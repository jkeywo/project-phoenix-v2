import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const WAV = readFileSync('../fixtures/audio-mono-right.wav');
const PAGE = mode => `<!doctype html><html><head><base href='/client/'><link rel='stylesheet' href='/client/gui/audio-settings.css'>
<style>body{margin:8px}main{max-width:40rem}</style></head><body><main></main><script type='module'>
import '/client/gui/strings-boot.js';
import { createHostAudio } from '/client/gui/host-audio.js';
import { createPrivateAudio } from '/client/gui/private-audio.js';
import { renderAudioSettingsPanel } from '/client/gui/audio-settings-panel.js';
import { createClientSemanticActionRegistry } from '/client/gui/client-semantic-actions.js';
import { loadOperatorProfile,saveOperatorProfile } from '/client/gui/operator-profile.js';
let context,master;const registry=createClientSemanticActionRegistry();
const contextFactory=()=>{context=new AudioContext();const create=context.createGain.bind(context);
  context.createGain=()=>{const node=create();master ||= node;return node;};return context;};
const load=()=>loadOperatorProfile(localStorage,{registry}).profile;
const options={contextFactory,fetchAudio:()=>fetch('/mono-right.wav')};
window.audio=${mode === 'room' ? 'createHostAudio({...options,storage:localStorage})' : 'createPrivateAudio({...options,read:()=>load().audio,save:audio=>saveOperatorProfile(localStorage,{...load(),audio})})'};
renderAudioSettingsPanel(document,document.querySelector('main'),audio);
window.start=async()=>{await audio.ready;await audio.enable();
  ${mode === 'room' ? "audio.audioConfig(JSON.stringify({ambient:{file:'assets/sounds/mono-fixture.wav',volume:0.5}}));audio.startGameAudio();" : ''}
  const split=context.createChannelSplitter(2);master.connect(split);
  const meters=[context.createAnalyser(),context.createAnalyser()];meters.forEach((meter,i)=>{meter.fftSize=1024;split.connect(meter,i);});
  window.samples=()=>{const values=meters.map(meter=>{const values=new Float32Array(1024);meter.getFloatTimeDomainData(values);return values;});
    const power=values.map(values=>Math.sqrt(values.reduce((sum,value)=>sum+value*value,0)/values.length));
    return {left:power[0],right:power[1],difference:Math.max(...values[0].map((value,i)=>Math.abs(value-values[1][i])))};};
};
</script></body></html>`;

for (const mode of ['room','private']) test(`${mode} mono controls actual right-only decoded PCM and persists locally`, {tag:'@core'}, async ({page})=>{
  const errors=[];page.on('pageerror',error=>errors.push(String(error)));
  await page.route('**/mono-output-probe',route=>route.fulfill({contentType:'text/html',body:PAGE(mode)}));
  await page.route('**/mono-right.wav',route=>route.fulfill({contentType:'audio/wav',body:WAV}));
  await page.goto('/mono-output-probe');
  const control=page.locator('[data-audio-mono]');await expect(control).toBeEnabled();await expect(control).not.toBeChecked();
  await page.evaluate(()=>window.start());
  await expect.poll(()=>page.evaluate(()=>audio.state().status)).not.toBe('loading');
  if(mode==='private') await page.evaluate(()=>audio.testOutput());
  await expect.poll(()=>page.evaluate(()=>samples().right)).toBeGreaterThan(0.001);
  expect(await page.evaluate(()=>samples().left)).toBeLessThan(0.00001);
  await control.focus();await page.keyboard.press('Space');await expect(control).toBeChecked();
  if(mode==='private') await page.evaluate(()=>audio.testOutput());
  await expect.poll(()=>page.evaluate(()=>samples().left)).toBeGreaterThan(0.001);
  await expect.poll(()=>page.evaluate(()=>samples().difference)).toBeLessThan(0.00001);
  expect(await control.evaluate(node=>node===document.activeElement)).toBe(true);
  await page.locator('[data-audio-bus=master] button').click();
  await expect.poll(()=>page.evaluate(()=>samples().right)).toBeLessThan(0.00001);
  await page.reload();await expect(control).toBeChecked();
  await page.locator('[data-audio-reset]').click();await expect(control).not.toBeChecked();
  await page.setViewportSize({width:390,height:844});await page.emulateMedia({forcedColors:'active'});
  await page.addStyleTag({content:'body{font-size:200%}'});await control.focus();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await expect(control).toBeInViewport();expect(errors).toEqual([]);
});
