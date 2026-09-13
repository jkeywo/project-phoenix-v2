import { test, expect } from '@playwright/test';

function wave(frequency, amplitude) {
  const rate = 44100, count = rate * 2, bytes = Buffer.alloc(44 + count * 2);
  bytes.write('RIFF'); bytes.writeUInt32LE(bytes.length - 8, 4); bytes.write('WAVEfmt ', 8);
  bytes.writeUInt32LE(16, 16); bytes.writeUInt16LE(1, 20); bytes.writeUInt16LE(1, 22);
  bytes.writeUInt32LE(rate, 24); bytes.writeUInt32LE(rate * 2, 28); bytes.writeUInt16LE(2, 32);
  bytes.writeUInt16LE(16, 34); bytes.write('data', 36); bytes.writeUInt32LE(count * 2, 40);
  for (let index = 0; index < count; index++) bytes.writeInt16LE(Math.round(amplitude * 32767
    * Math.sin(index * frequency * 2 * Math.PI / rate)), 44 + index * 2);
  return bytes;
}
const HTML = `<!doctype html><html><head><meta name="viewport" content="width=device-width">
<link rel="stylesheet" href="/client/gui/audio-settings.css"></head><body><main></main><script type="module">
import '/client/gui/strings-boot.js';
import {createHostAudio} from '/client/gui/host-audio.js';
import {renderAudioSettingsPanel} from '/client/gui/audio-settings-panel.js';
window.audio=createHostAudio({storage:localStorage});
audio.audioConfig(JSON.stringify({ambient:{file:'assets/sounds/duck-bed.wav',volume:0.25},
 engine:{file:'assets/sounds/duck-bed.wav',idle_volume:0.25,volume_at_full_thrust:0},
 red_alert:{siren_file:'assets/sounds/duck-alert.wav',siren_volume:0.7,music_file:'assets/sounds/duck-bed.wav',music_volume:0},
 computer_message:{critical:{file:'assets/sounds/duck-alert.wav',volume:0.7}}}));
audio.startGameAudio(); audio.applyHudAudio({red_alert:false});
renderAudioSettingsPanel(document,document.querySelector('main'),audio);
window.peak=async()=>{let value=0;for(let i=0;i<8;i++){await new Promise(requestAnimationFrame);value=Math.max(value,audio.debug().outputPeak);}return value;};
</script></body></html>`;

test('real decoded room samples duck on red alert and authored-shaped alert then recover with accessible live controls', { tag: '@core' }, async ({ page }) => {
  await page.route('**/client/ducking-probe', route => route.fulfill({contentType:'text/html',body:HTML}));
  await page.route('**/assets/sounds/duck-bed.wav', route => route.fulfill({contentType:'audio/wav',body:wave(440,0.1)}));
  await page.route('**/assets/sounds/duck-alert.wav', route => route.fulfill({contentType:'audio/wav',body:wave(1700,0.003)}));
  const errors=[];page.on('pageerror',error=>errors.push(String(error)));
  await page.goto('/client/ducking-probe');
  await expect(page.locator('[data-audio-ducking]')).toBeEnabled();
  await page.locator('[data-audio-enable]').click();
  await expect.poll(()=>page.evaluate(()=>audio.state().ready.includes('siren'))).toBe(true);
  const base=await page.evaluate(()=>peak());expect(base).toBeGreaterThan(0.03);
  await page.locator('[data-audio-ducking]').focus();await page.keyboard.press('Space');
  await page.evaluate(()=>audio.applyHudAudio({red_alert:true}));
  await page.waitForTimeout(120);
  expect(await page.evaluate(()=>peak())).toBeLessThan(base*0.55);
  // A real authored-shaped computer Alert takes the same provider path and
  // extends this current envelope; it never adds a retained alert queue.
  await page.evaluate(()=>audio.audioCue(JSON.stringify({kind:'computer_message',severity:'critical'})));
  await page.waitForTimeout(2600);
  expect(await page.evaluate(()=>peak())).toBeGreaterThan(base*0.9);
  await page.evaluate(()=>audio.audioCue(JSON.stringify({kind:'computer_message',severity:'critical'})));
  await page.waitForTimeout(120);
  expect(await page.evaluate(()=>peak())).toBeLessThan(base*0.55);
  await page.locator('[data-audio-bus="ambience"] button').click();
  await page.waitForTimeout(120);
  expect(await page.evaluate(()=>peak())).toBeLessThan(0.005);
  await page.setViewportSize({width:390,height:844});await page.emulateMedia({forcedColors:'active'});
  await page.addStyleTag({content:'body{font-size:200%;}'});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  const saved=await page.evaluate(()=>JSON.parse(localStorage.getItem('phoenix-viewscreen-presentation-v1')));
  expect(saved.audio.ducking).toBe(true);
  expect(errors).toEqual([]);
});
