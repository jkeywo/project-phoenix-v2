import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import path from 'node:path';

const policies = JSON.parse(readFileSync(path.join(__dirname, '../fixtures/audio-spatial-policy.json'), 'utf8'));
const reading = JSON.parse(readFileSync(path.join(__dirname, '../fixtures/native-hud-reading.json'), 'utf8'));

// The real Display controls send their typed record. These bridge replies are
// shared with the Rust host-record/cache test, rather than resolving preferences
// a second time in this smoke harness. The child is the actual native HUD.
const DISPLAY_PAGE = `<!doctype html><html><head>
<link rel="stylesheet" href="/gui/tokens.css"><link rel="stylesheet" href="/gui/native-settings.css">
</head><body><iframe id="hud" src="/gui/viewscreen-hud.html" style="width:1100px;height:720px"></iframe>
<script type="module">
import '/gui/strings-boot.js';
import { mountNativeSettings } from '/gui/native-settings.js';
import { createViewscreenPresentation, viewscreenPresentationRecordFields } from '/gui/viewscreen-presentation.js';
window.PhoenixOsAccessibilityDefaults = {textScale:1.25,contrast:false};
const fixtures = ${JSON.stringify(reading)};
window.requests = []; window.unmatched = [];
const presentation = createViewscreenPresentation({doc:document,win:window,store:{
  load:()=>({}), save:record=>{
    const request = {kind:'set_presentation',...viewscreenPresentationRecordFields(record)};
    window.requests.push(request);
    const fixture = fixtures.find(f=>Object.entries(f.record).every(([key,value])=>request[key]===value));
    if(fixture) document.querySelector('#hud').contentWindow.eval(fixture.script);
    else window.unmatched.push(request);
  }
}});
presentation.apply();
window.shell = mountNativeSettings(document,{presentation});
</script></body></html>`;

test('native combat policy matches real browser PannerNode samples @core', async ({ page }) => {
  await page.goto('/gui/viewscreen-hud.html');
  const actual = await page.evaluate(async policies => {
    const output = [];
    for (const { position, samples, spec } of policies) {
      const context = new OfflineAudioContext(2, 512, 44100);
      const source = context.createBufferSource();
      source.buffer = context.createBuffer(samples.length, 512, 44100);
      samples.forEach((value, channel) => source.buffer.getChannelData(channel).fill(value));
      const panner = new PannerNode(context, {
        panningModel: spec.panning_model, distanceModel: spec.distance_model,
        refDistance: spec.ref_distance, maxDistance: spec.max_distance, rolloffFactor: spec.rolloff_factor,
        positionX: position[0], positionY: position[1], positionZ: position[2],
      });
      source.connect(panner).connect(context.destination); source.start();
      const rendered = await context.startRendering();
      output.push([rendered.getChannelData(0)[256], rendered.getChannelData(1)[256]]);
    }
    return output;
  }, policies);
  actual.forEach((channels, index) => channels.forEach((sample, channel) => {
    expect(sample, policies[index].name).toBeCloseTo(policies[index].expected[channel], 5);
  }));
});

test('native HUD keeps current informative cues readable without an audio device @core', async ({ page }) => {
  await page.goto('/gui/viewscreen-hud.html');
  await page.waitForFunction(() => !!window.__phoenixHud?.audioEquivalent);
  await page.evaluate(() => {
    window.__phoenixHudAudioCue(JSON.stringify({ kind: 'lifecycle', active: true }));
    window.__updateHud(JSON.stringify({ heading: 0, hull_pct: 90, red_alert: true, phaser_firing: true, condition: 'server.hud_alert' }));
    window.__phoenixHudAudioCue(JSON.stringify({ kind: 'beam', active: true }));
    window.__phoenixSetHudEffects({ shake: 0, flash: 0, decorativeMotion: 0 });
    window.__phoenixHudAudioCue(JSON.stringify({ kind: 'blaster', x: 30, y: 0, z: 0 }));
  });
  await expect(page.locator('[data-audio-equivalent="blaster"]')).toContainText('90');
  await expect(page.locator('[data-audio-equivalent="beam"]')).toBeVisible();
  await page.emulateMedia({ forcedColors: 'active' });
  await expect(page.locator('[data-audio-equivalent="beam"]')).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-flash', 'off');
  await page.evaluate(() => window.__phoenixHudAudioCue(JSON.stringify({ kind: 'lifecycle', active: false })));
  await expect(page.locator('#audio-live-equivalents')).toBeEmpty();
});

test('native Display controls scale and contrast both live audio and presentation text @core', async ({ page }) => {
  const errors = []; page.on('pageerror', error => errors.push(String(error)));
  await page.route('**/native-hud-display-probe', route => route.fulfill({contentType:'text/html',body:DISPLAY_PAGE}));
  await page.goto('/native-hud-display-probe');
  const hud = page.frameLocator('#hud');
  await expect(hud.locator('#audio-live-equivalents')).toBeAttached();
  await page.evaluate(script => {
    const child = document.querySelector('#hud').contentWindow;
    child.eval(script);
    child.__phoenixHudAudioCue(JSON.stringify({kind:'lifecycle',active:true}));
    child.__phoenixHudAudioCue(JSON.stringify({kind:'beam',active:true}));
    child.__updateHud(JSON.stringify({presentation_card:{kind:'title',title:'Current mission',body:'Maintain the beam.'}}));
    window.shell.open(); window.shell.selectTab('presentation');
  }, reading[0].script);
  const beam = hud.locator('[data-audio-equivalent="beam"]');
  const card = hud.locator('.vs-presentation-card');
  const title = card.locator('h1');
  const fontSize = locator => locator.evaluate(el => parseFloat(getComputedStyle(el).fontSize));
  const background = locator => locator.evaluate(el => getComputedStyle(el).backgroundColor);
  const baseline = [await fontSize(beam), await fontSize(title)];
  const colours = [await background(beam), await background(card)];
  const slider = page.locator('[data-control="viewscreen-text-scale"]');
  await slider.focus(); await page.keyboard.press('End');
  await expect(slider).toHaveValue('2');
  expect(await fontSize(beam)).toBeCloseTo(baseline[0] * 1.6, 4);
  expect(await fontSize(title)).toBeCloseTo(baseline[1] * 1.6, 4);
  await page.locator('[data-control="viewscreen-contrast-on"]').click();
  for (const [index, locator] of [beam, card].entries()) {
    expect(await background(locator)).not.toBe(colours[index]);
    expect(await background(locator)).toBe('rgb(0, 0, 0)');
    expect(await locator.evaluate(el => getComputedStyle(el).color)).toBe('rgb(255, 255, 255)');
  }
  // The card must not paint over the simultaneous informative audio equivalent.
  expect(await beam.evaluate(el => {
    const r = el.getBoundingClientRect();
    el.style.pointerEvents = 'auto';
    const visible = el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
    el.style.pointerEvents = '';
    return visible;
  })).toBe(true);
  await page.locator('[data-control="viewscreen-text-scale-reset"]').click();
  expect(await fontSize(beam)).toBeCloseTo(baseline[0], 4);
  expect(await fontSize(title)).toBeCloseTo(baseline[1], 4);
  await expect(hud.locator('html')).toHaveAttribute('data-contrast', 'more');
  expect(await page.evaluate(() => window.unmatched)).toEqual([]);
  expect(await page.evaluate(() => window.requests.length)).toBe(3);
  expect(errors).toEqual([]);
});

test('native computer text remains current, scaled and explicit while room sound is unavailable @core', async ({ page }) => {
  await page.route('**/native-hud-display-probe', route => route.fulfill({contentType:'text/html',body:DISPLAY_PAGE}));
  await page.goto('/native-hud-display-probe');
  const hud = page.frameLocator('#hud');
  await expect(hud.locator('#hud-computer-message')).toBeAttached();
  await page.evaluate(script => {
    const child = document.querySelector('#hud').contentWindow;
    child.eval(script);
    child.__updateHud(JSON.stringify({computer_message:{text:'Maintain course.',severity:'advisory',station:'helm'},
      presentation_card:{kind:'title',title:'Incoming',body:'Current presentation.'}}));
    window.shell.open(); window.shell.selectTab('presentation');
  }, reading[0].script);
  const banner = hud.locator('#hud-computer-message');
  const text = hud.locator('#hud-computer-message-text');
  await expect(banner).toBeVisible();
  await expect(banner).toContainText('Advisory');
  await expect(banner).toContainText('Helm');
  const base = await text.evaluate(el => parseFloat(getComputedStyle(el).fontSize));
  await page.locator('[data-control="viewscreen-text-scale"]').focus();
  await page.keyboard.press('End');
  expect(await text.evaluate(el => parseFloat(getComputedStyle(el).fontSize))).toBeCloseTo(base * 1.6, 4);
  await page.locator('[data-control="viewscreen-contrast-on"]').click();
  expect(await banner.evaluate(el => getComputedStyle(el).backgroundColor)).toBe('rgb(0, 0, 0)');
  expect(await text.evaluate(el => getComputedStyle(el).color)).toBe('rgb(255, 255, 255)');
  expect(await banner.evaluate(el => getComputedStyle(el).animationName)).toBe('none');
  expect(await banner.evaluate(el => {
    const r=el.getBoundingClientRect(); el.style.pointerEvents='auto';
    const top=el.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2));
    el.style.pointerEvents=''; return top;
  })).toBe(true);
  await page.evaluate(() => document.querySelector('#hud').contentWindow.__updateHud(JSON.stringify({
    computer_message:{text:'Replacement message.',severity:'critical',station:'unknown-station'}
  })));
  await expect(text).toHaveText('Replacement message.');
  await expect(banner).toContainText('Critical');
  await expect(hud.locator('#hud-computer-message-station')).toBeEmpty();
  await page.evaluate(() => document.querySelector('#hud').contentWindow.__updateHud(JSON.stringify({computer_message:null})));
  await expect(banner).toBeHidden(); await expect(text).toBeEmpty();
  await expect(hud.locator('.vs-computer-message')).toHaveCount(1);
});
