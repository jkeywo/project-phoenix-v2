import { test, expect } from '@playwright/test';

// A deliberate real-browser provider probe. No WASM or fake audio element:
// production settings/consumer/provider decode the shipped MP3 and OGG assets.
// The existing audio.spec.js separately exercises the TOML→host-channel route.
const PAGE = `<!doctype html><html><head><link rel="stylesheet" href="/gui/audio-settings.css"></head>
<body><main id="settings"></main><script type="module">
import '/gui/strings-boot.js';
import { createHostAudio } from '/gui/host-audio.js';
import { renderAudioSettingsPanel } from '/gui/audio-settings-panel.js';
import { createAudioLiveEquivalents } from '/gui/audio-live-equivalents.js';
const equivalents = createAudioLiveEquivalents(document);
window.audio = createHostAudio({doc: document, storage: localStorage, onEquivalent: equivalents.update});
renderAudioSettingsPanel(document, document.querySelector('main'), window.audio);
</script></body></html>`;

test('Viewscreen mixer controls real decoded samples, live mute, persistence and equivalents', { tag: '@core' }, async ({ page }) => {
  await page.route('**/audio-output-probe', route => route.fulfill({ contentType: 'text/html', body: PAGE }));
  const errors = [];
  page.on('pageerror', error => errors.push(String(error)));
  await page.goto('/audio-output-probe');
  const master = page.locator('[data-audio-bus="master"] input');
  const musicMute = page.locator('[data-audio-bus="music"] button');
  await expect(master).toBeVisible();
  await master.focus();
  await page.keyboard.press('ArrowLeft');
  await expect(master).toHaveValue('0.99');
  await page.locator('[data-audio-test]').click();
  await expect.poll(() => page.evaluate(() => window.audio.debug().outputPeak)).toBeGreaterThan(0.001);
  await musicMute.click();
  await expect.poll(() => page.evaluate(() => window.audio.debug().outputPeak)).toBeLessThan(0.00001);
  await expect(musicMute).toHaveAttribute('aria-pressed', 'true');
  await page.reload();
  await expect(master).toHaveValue('0.99');
  await expect(musicMute).toHaveAttribute('aria-pressed', 'true');
  await page.evaluate(() => {
    window.audio.audioConfig(JSON.stringify({
      blaster: { file: 'assets/sounds/Blaster.mp3', volume: 0.9, ref_distance: 30, max_distance: 800,
        rolloff_factor: 1.2, distance_model: 'inverse', panning_model: 'equalpower' },
      red_alert: { siren_file: 'assets/sounds/red_alert_siren.ogg', siren_volume: 0.7 },
    }));
    window.audio.startGameAudio();
  });
  await page.locator('[data-audio-enable]').click();
  await expect.poll(() => page.evaluate(() => window.audio.state().ready.includes('siren'))).toBe(true);
  await page.evaluate(() => {
    window.audio.applyHudAudio({ red_alert: false });
    window.audio.applyHudAudio({ red_alert: true });
  });
  await expect.poll(() => page.evaluate(() => window.audio.debug().outputPeak)).toBeGreaterThan(0.001);
  await page.locator('[data-audio-bus="master"] button').click();
  await expect.poll(() => page.evaluate(() => window.audio.debug().outputPeak)).toBeLessThan(0.00001);
  await page.evaluate(() => window.audio.audioCue(JSON.stringify({ kind: 'blaster', x: 10, y: 0, z: 0 })));
  await expect(page.locator('[data-audio-equivalent="blaster"]')).toContainText('90°');
  await page.evaluate(() => window.audio.resetSession());
  await expect(page.locator('[data-audio-equivalent]')).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => window.audio.state().active.length)).toBe(0);
  await page.locator('[data-audio-reset]').click();
  await expect(master).toHaveValue('1');
  await expect(musicMute).toHaveAttribute('aria-pressed', 'false');
  // Supported scale/contrast keeps each control reachable without horizontal
  // scrolling; keyboard changes above travelled through the actual input event.
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ forcedColors: 'active' });
  await page.addStyleTag({ content: 'body {font-size: 200%;}' });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await expect(page.locator('[data-audio-test]')).toBeVisible();
  expect(errors).toEqual([]);
});
