import { test, expect } from '@playwright/test';

function wave() {
  const rate=24000, frames=rate*5, bytes=Buffer.alloc(44+frames*4);
  bytes.write('RIFF');bytes.writeUInt32LE(bytes.length-8,4);bytes.write('WAVEfmt ',8);
  bytes.writeUInt32LE(16,16);bytes.writeUInt16LE(1,20);bytes.writeUInt16LE(2,22);
  bytes.writeUInt32LE(rate,24);bytes.writeUInt32LE(rate*4,28);bytes.writeUInt16LE(4,32);bytes.writeUInt16LE(16,34);
  bytes.write('data',36);bytes.writeUInt32LE(frames*4,40);
  for(let i=0;i<frames;i++) {
    const amplitude=i>=rate&&i<rate*2?0.8:0.025, value=Math.sin(i*2*Math.PI*440/rate)*amplitude;
    bytes.writeInt16LE(Math.round(value*32767),44+i*4);bytes.writeInt16LE(Math.round(value*0.4*32767),46+i*4);
  }
  return bytes;
}
const WAVE=wave();
const PULSE=Buffer.from(WAVE.subarray(0,44+128*4));
PULSE.writeUInt32LE(PULSE.length-8,4);PULSE.writeUInt32LE(PULSE.length-44,40);
const OFFLINE=`<!doctype html><script type='module'>
import {createBrowserAudioProvider} from '/client/gui/browser-audio-provider.js';
import {defaultAudioMix} from '/client/gui/audio-mix.js';
window.render=async(reduced,master=1,muted=false)=>{
 const context=new OfflineAudioContext(2,24000*5,24000);
 Object.defineProperty(context,'state',{get:()=> 'running'});context.close=async()=>{};
 const provider=createBrowserAudioProvider({contextFactory:()=>context,fetchAudio:()=>fetch('/range.wav')});
 const mix=defaultAudioMix();mix.master.level=master;mix.effects.muted=muted;provider.setMix(mix);provider.setReducedRange(reduced);
 provider.register('cue',{file:'range.wav',volume:1,category:'effects'});
 while(!provider.snapshot().ready.includes('cue')) await new Promise(resolve=>setTimeout(resolve,0));
 provider.cue('cue');const output=await context.startRendering();
 const samples=output.getChannelData(0),right=output.getChannelData(1);
 const rms=(a,b)=>Math.sqrt(samples.slice(a*24000,b*24000).reduce((v,x)=>v+x*x,0)/((b-a)*24000));
 const result={quiet:rms(.4,.8),loud:rms(1.4,1.8),recovered:rms(4.2,4.8),peak:samples.reduce((v,x)=>Math.max(v,Math.abs(x)),0),
 finite:samples.every(Number.isFinite)&&right.every(Number.isFinite),rightEnergy:right.reduce((v,x)=>v+x*x,0),available:provider.snapshot().reducedRangeAvailable};
 provider.dispose();return result;
};
window.pulse=async(bus=null,zero=false,auditionStop=null)=>{
 const context=new OfflineAudioContext(2,2400,24000);
 Object.defineProperty(context,'state',{get:()=> 'running'});context.close=async()=>{};
 const provider=createBrowserAudioProvider({contextFactory:()=>context,fetchAudio:()=>fetch('/pulse.wav')});
 provider.setReducedRange(true);
 if(auditionStop!==null)await provider.audition({file:'pulse.wav',volume:1,category:'effects'});
 else {provider.register('pulse',{file:'pulse.wav',volume:1,category:'effects'});
   while(!provider.snapshot().ready.includes('pulse')) await new Promise(resolve=>setTimeout(resolve,0));
   provider.cue('pulse');}
 const paused=context.suspend(128/24000),rendering=context.startRendering();await paused;
 if(bus){const mix=defaultAudioMix();if(zero)mix[bus].level=0;else mix[bus].muted=true;
   provider.setMix(mix);provider.setMix(defaultAudioMix());}
 if(auditionStop===true)provider.stopAudition();
 await context.resume();const output=await rendering;
 const peak=output.getChannelData(0).slice(128).reduce((v,x)=>Math.max(v,Math.abs(x)),0);
 provider.dispose();return peak;
};</script>`;

test('production browser range path lifts quiet samples, lowers loud samples and keeps final Master linear', {tag:'@core'}, async({page})=>{
  await page.route('**/range-offline',route=>route.fulfill({contentType:'text/html',body:OFFLINE}));
  await page.route('**/range.wav',route=>route.fulfill({contentType:'audio/wav',body:WAVE}));
  await page.goto('/range-offline');await page.waitForFunction(()=>typeof render==='function');
  const full=await page.evaluate(()=>render(false)),reduced=await page.evaluate(()=>render(true));
  expect(full.peak).toBeCloseTo(.8,3);expect(reduced.available).toBe(true);expect(reduced.finite).toBe(true);
  expect(reduced.quiet).toBeGreaterThan(full.quiet*1.1);expect(reduced.quiet).toBeLessThan(full.quiet*2);
  expect(reduced.loud).toBeLessThan(full.loud*.6);expect(reduced.loud/reduced.quiet).toBeLessThan(full.loud/full.quiet*.4);
  expect(reduced.peak).toBeLessThanOrEqual(.50001);expect(reduced.recovered/reduced.quiet).toBeCloseTo(1,2);
  const lower=await page.evaluate(()=>render(true,.25));expect(lower.quiet).toBeCloseTo(reduced.quiet*.25,6);expect(lower.loud).toBeCloseTo(reduced.loud*.25,6);
  expect((await page.evaluate(()=>render(true,0))).peak).toBe(0);
  expect((await page.evaluate(()=>render(true,1,true))).peak).toBe(0);
});

test('a quick Master/category mute or zero discards real compressor lookahead instead of replaying it', {tag:'@core'}, async({page})=>{
  await page.route('**/range-offline',route=>route.fulfill({contentType:'text/html',body:OFFLINE}));
  await page.route('**/pulse.wav',route=>route.fulfill({contentType:'audio/wav',body:PULSE}));
  await page.goto('/range-offline');await page.waitForFunction(()=>typeof pulse==='function');
  // The decoded source has ended at the boundary; only the real processor's
  // delayed samples remain. No sample-time elapses between mute and unmute.
  expect(await page.evaluate(()=>pulse())).toBeGreaterThan(.0001);
  for(const bus of ['master','effects'])for(const zero of [false,true])
    expect(await page.evaluate(({bus,zero})=>pulse(bus,zero),{bus,zero})).toBe(0);
});

test('explicit audition Stop discards real compressor lookahead immediately', {tag:'@core'}, async({page})=>{
  await page.route('**/range-offline',route=>route.fulfill({contentType:'text/html',body:OFFLINE}));
  await page.route('**/pulse.wav',route=>route.fulfill({contentType:'audio/wav',body:PULSE}));
  await page.goto('/range-offline');await page.waitForFunction(()=>typeof pulse==='function');
  expect(await page.evaluate(()=>pulse(null,false,false))).toBeGreaterThan(.0001);
  expect(await page.evaluate(()=>pulse(null,false,true))).toBe(0);
});

const PAGE=mode=>`<!doctype html><html><head><base href='/client/'><meta name='viewport' content='width=device-width'>
<link rel='stylesheet' href='/client/gui/audio-settings.css'><style>body{margin:8px}</style></head><body><main></main><script type='module'>
import '/client/gui/strings-boot.js';
import {createHostAudio} from '/client/gui/host-audio.js';import {createPrivateAudio} from '/client/gui/private-audio.js';
import {renderAudioSettingsPanel} from '/client/gui/audio-settings-panel.js';
import {createClientSemanticActionRegistry} from '/client/gui/client-semantic-actions.js';
import {loadOperatorProfile,saveOperatorProfile} from '/client/gui/operator-profile.js';
const registry=createClientSemanticActionRegistry(),load=()=>loadOperatorProfile(localStorage,{registry}).profile;
let starts=0;const options={fetchAudio:()=>fetch('/range.wav'),contextFactory:()=>{const context=new AudioContext();
 const create=context.createBufferSource.bind(context);context.createBufferSource=()=>{const source=create(),start=source.start.bind(source);source.start=(...args)=>{starts++;return start(...args);};return source;};return context;}};
window.audio=${mode==='room'?'createHostAudio({...options,storage:localStorage})':'createPrivateAudio({...options,read:()=>load().audio,save:audio=>saveOperatorProfile(localStorage,{...load(),audio})})'};
renderAudioSettingsPanel(document,document.querySelector('main'),audio);window.starts=()=>starts;
window.start=async()=>{await audio.ready;await audio.enable();${mode==='room'?'audio.audioConfig(JSON.stringify({ambient:{file:"range.wav",volume:1}}));audio.startGameAudio();':'audio.actionable();'}};
</script></body></html>`;
for(const mode of ['room','private'])test(`${mode} range control processes current sources without restart and persists the local choice`,{tag:'@core'},async({page})=>{
  const errors=[];page.on('pageerror',error=>errors.push(String(error)));
  await page.route('**/range-control',route=>route.fulfill({contentType:'text/html',body:PAGE(mode)}));
  await page.route('**/range.wav',route=>route.fulfill({contentType:'audio/wav',body:WAVE}));
  await page.goto('/range-control');const range=page.locator('[data-audio-range]');
  await expect(range).toBeEnabled();await expect(range).not.toBeChecked();
  await page.evaluate(()=>window.start());
  await expect.poll(()=>page.evaluate(()=>audio.state().status)).not.toBe('loading');
  if(mode==='private')await page.evaluate(()=>audio.actionable());
  await expect.poll(()=>page.evaluate(()=>audio.debug().outputPeak)).toBeGreaterThan(.0001);
  const starts=await page.evaluate(()=>window.starts());await range.focus();await page.keyboard.press('Space');
  await expect(range).toBeChecked();expect(await page.evaluate(()=>window.starts())).toBe(starts);
  await page.locator('[data-audio-bus=master] button').click();
  await expect.poll(()=>page.evaluate(()=>audio.debug().outputPeak)).toBeLessThan(.00001);
  await page.reload();await expect(range).toBeChecked();await page.locator('[data-audio-reset]').click();await expect(range).not.toBeChecked();
  await page.setViewportSize({width:390,height:844});await page.emulateMedia({forcedColors:'active'});await page.addStyleTag({content:'body{font-size:200%}'});
  await range.focus();expect(await range.evaluate(node=>node===document.activeElement)).toBe(true);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);await expect(range).toBeInViewport();expect(errors).toEqual([]);
});
