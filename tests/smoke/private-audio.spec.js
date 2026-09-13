import { test, expect } from '@playwright/test';

// Real shared Station adapter, iframe bridge, private owner and browser decoder.
// The fixture stops at command transport: the correlated answer is supplied by
// the test, while existing semantic action smokes cover live Admission.
const STATION = `<!doctype html><html><head><link rel="stylesheet" href="/client/gui/audio-settings.css"></head><body>
<main></main><iframe id="station"></iframe><script type="module">
import '/client/gui/strings-boot.js';
import { createPrivateAudio, privateFeedbackReceiver } from '/client/gui/private-audio.js';
import { renderAudioSettingsPanel } from '/client/gui/audio-settings-panel.js';
window.audio = createPrivateAudio({root:window});
window.sent = [];
const frame = document.getElementById('station');
window.addEventListener('message', event => {if(event.source===frame.contentWindow && event.data?.type==='console_action') sent.push(JSON.parse(event.data.payload));});
window.__privateActionFeedback = privateFeedbackReceiver({getAudio:()=>audio,currentSource:()=>frame.contentWindow});
renderAudioSettingsPanel(document, document.querySelector('main'), audio);
frame.srcdoc = '<script type="module">import {initConsole} from "/client/gui/console-core.js"; window.__sendAction = json => parent.sent.push(JSON.parse(json)); window.runtime = initConsole({name:"captain",render:()=>{}}); window.__updateConsole("captain", JSON.stringify({red_alert:false,red_alert_auto:false}));<\\/script>';
</script></body></html>`;

const GM = `<!doctype html><html><head><link rel="stylesheet" href="/client/gui/audio-settings.css"></head><body>
<section id="gm-mission-panel"></section><main id="audio"></main><script type="module">
import '/client/gui/strings-boot.js';
import { mountGmWorkspace } from '/client/gui/gm-workspace.js';
import { renderAudioSettingsPanel } from '/client/gui/audio-settings-panel.js';
import { createHostAudio } from '/client/gui/host-audio.js';
window.__phoenixGmPage = true; window.__hostLocalGm = () => ({id:'gm-one'}); window.sent=[];
window.__hostPresentation = request => {sent.push(request);return true;};
window.workspace = mountGmWorkspace({win:window}); window.audio = window.__privateAudio;
window.room = createHostAudio({doc:document,isRoom:()=>!window.__phoenixGmPage});
room.audioConfig(JSON.stringify({ambient:{file:'assets/sounds/Ambient.mp3',volume:0.5}}));room.startGameAudio();
workspace.handlers.gm_entity({entities:[{entity_id:'owned-ship',kind:'player_ship',name:'Owned ship'}]});
renderAudioSettingsPanel(document, document.getElementById('audio'), audio);
</script></body></html>`;

test('private Station action and refusal reach decoded output once through its current iframe', { tag: '@core' }, async ({ page }) => {
  await page.route('**/client/private-station-probe', route => route.fulfill({ contentType: 'text/html', body: STATION }));
  const errors = []; page.on('pageerror', error => errors.push(String(error)));
  await page.goto('/client/private-station-probe');
  await expect.poll(() => page.evaluate(() => window.audio?.state().ready.length)).toBe(7);
  await expect.poll(() => page.evaluate(() => !!document.querySelector('iframe').contentWindow.activateSemanticAction)).toBe(true);
  expect(await page.locator('[data-audio-bus]').count()).toBe(3);
  await page.locator('[data-audio-enable]').click();
  const played = await page.evaluate(async () => {
    const child = document.querySelector('iframe').contentWindow;
    child.activateSemanticAction('captain.red-alert', {context:'captain',source:'control'});
    await new Promise(resolve => setTimeout(resolve, 25)); return audio.debug().outputPeak;
  });
  expect(played).toBeGreaterThan(0.00001);
  expect(await page.evaluate(() => sent[0].action)).toBe('set_red_alert');
  expect(await page.evaluate(async () => {
    const child = document.querySelector('iframe').contentWindow;
    child.__updateActionFeedback({correlation:sent[0].correlation,state:'Refused'});
    await new Promise(resolve => setTimeout(resolve, 30)); return audio.debug().outputPeak;
  })).toBeGreaterThan(0.00001);
  await expect.poll(() => page.evaluate(() => audio.state().active.length)).toBe(0);
  await page.evaluate(() => document.querySelector('iframe').contentWindow.__updateActionFeedback({correlation:sent[0].correlation,state:'Refused'}));
  expect(await page.evaluate(() => audio.state().active.length)).toBe(0);
  await page.locator('[data-audio-bus="master"] button').click();
  await page.locator('[data-audio-test]').click();
  expect(await page.evaluate(() => audio.state().active.length)).toBe(0);
  await page.setViewportSize({width:390,height:844}); await page.emulateMedia({forcedColors:'active'});
  await page.addStyleTag({content:'body {font-size:200%;} iframe {max-width:100%;}'});
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  expect(errors).toEqual([]);
});

test('private GM M8 action settles only its own journal result and owned ship never starts room audio', { tag: '@core' }, async ({ page }) => {
  await page.route('**/client/private-gm-probe', route => route.fulfill({ contentType: 'text/html', body: GM }));
  const errors = []; page.on('pageerror', error => errors.push(String(error)));
  await page.goto('/client/private-gm-probe');
  await expect.poll(() => page.evaluate(() => window.audio?.state().ready.length)).toBe(7);
  await page.locator('#audio [data-audio-enable]').click();
  await page.locator('#gm-presentation-panel > button').nth(1).click();
  expect(await page.evaluate(() => sent[0].cue)).toBe('release_view');
  await expect.poll(() => page.evaluate(() => audio.state().active.length)).toBe(0);
  await page.evaluate(() => workspace.handlers.gm_session({journal:{entries:[{operator_id:'other',correlation:sent[0].correlation,outcome:'refused'}]}}));
  expect(await page.evaluate(() => audio.state().active.length)).toBe(0);
  expect(await page.evaluate(async () => {
    workspace.handlers.gm_session({journal:{entries:[{operator_id:'gm-one',correlation:sent[0].correlation,outcome:'refused'}]}});
    await new Promise(resolve => setTimeout(resolve,30)); return audio.debug().outputPeak;
  })).toBeGreaterThan(0.00001);
  expect(await page.evaluate(() => room.state().active)).toEqual([]);
  // GM exposes the authored audition categories locally; owning a ship still
  // never activates the room soundtrack (the production room owner above).
  expect(await page.evaluate(() => ({ categories: audio.state().categories,
    audition: audio.state().auditionAvailable, room: audio.state().room })))
    .toEqual({ categories: ['music','ambience','effects','alerts','interface'], audition: true, room: false });
  expect(errors).toEqual([]);
});
