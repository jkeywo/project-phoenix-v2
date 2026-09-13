import { test, expect } from '@playwright/test';

function wave(value, frames = 2048) {
  const bytes = Buffer.alloc(44 + frames * 2);
  bytes.write('RIFF'); bytes.writeUInt32LE(bytes.length - 8, 4); bytes.write('WAVEfmt ', 8);
  bytes.writeUInt32LE(16, 16); bytes.writeUInt16LE(1, 20); bytes.writeUInt16LE(1, 22);
  bytes.writeUInt32LE(24000, 24); bytes.writeUInt32LE(48000, 28); bytes.writeUInt16LE(2, 32);
  bytes.writeUInt16LE(16, 34); bytes.write('data', 36); bytes.writeUInt32LE(frames * 2, 40);
  for (let i = 0; i < frames; i++) bytes.writeInt16LE(value, 44 + i * 2);
  return Array.from(bytes);
}
const PAGE = `<!doctype html><script type='module'>
import {createBrowserAudioProvider} from '/client/gui/browser-audio-provider.js';
window.renderRevision = async (initial, replacement, pulse) => {
 let revision=1, bytes=new Uint8Array(initial);
 window.__hostPackRevision=()=>revision; window.__hostReadPackAsset=()=>bytes;
 const context=new OfflineAudioContext(2,512,24000);
 Object.defineProperty(context,'state',{get:()=> 'running'});context.close=async()=>{};
 const provider=createBrowserAudioProvider({contextFactory:()=>context});
 const ready=async()=>{while(!provider.snapshot().ready.includes('bed'))await new Promise(resolve=>setTimeout(resolve,0));};
 provider.register('bed',{file:'epoch.wav',category:'ambience',volume:1,loop:!pulse});
 await ready();
 if(pulse){provider.setReducedRange(true);provider.cue('bed');}
 else {
  provider.loop('bed',true);
  provider.register('shot',{file:'epoch.wav',category:'effects',volume:1});
  while(!provider.snapshot().ready.includes('shot'))await new Promise(resolve=>setTimeout(resolve,0));
  provider.cue('shot');await provider.testOutput('shot');
  await provider.audition({file:'epoch.wav',category:'effects',volume:1});
 }
 const first=context.suspend(128/24000),second=!pulse?context.suspend(256/24000):null;
 const rendering=context.startRendering();await first;
 bytes=new Uint8Array(replacement);revision++;
 await ready();const after=provider.snapshot();
 await context.resume();
 if(second){await second;bytes=null;revision++;await ready();await context.resume();}
 const output=await rendering,left=Array.from(output.getChannelData(0)),right=Array.from(output.getChannelData(1));
 provider.dispose();return {left,right,after};
};</script>`;

test('accepted replacement and removal decode real WAV bytes while only current loops return', {tag:'@core'}, async({page})=>{
 await page.route('**/audio-epoch',route=>route.fulfill({contentType:'text/html',body:PAGE}));
 let fallback=0;
 await page.route('**/epoch.wav',route=>{fallback++;return route.fulfill({contentType:'audio/wav',body:Buffer.from(wave(4096))});});
 await page.goto('/audio-epoch');await page.waitForFunction(()=>typeof renderRevision==='function');
 const result=await page.evaluate(({first,next})=>renderRevision(first,next,false),{first:wave(8192),next:wave(16384)});
 expect(result.left.every(Number.isFinite)).toBe(true);expect(result.left).toEqual(result.right);
 for(const value of result.left.slice(0,128))expect(value).toBeCloseTo(4*8192/32767,5);
 for(const value of result.left.slice(128,256))expect(value).toBeCloseTo(16384/32767,5);
 for(const value of result.left.slice(256))expect(value).toBeCloseTo(4096/32767,5);
 expect(result.after.active.map(voice=>voice.id)).toEqual(['bed']);expect(result.after.test).toBe('idle');
 expect(fallback).toBe(1);
});

test('an asset boundary also discards the real range processor lookahead', {tag:'@core'}, async({page})=>{
 await page.route('**/audio-epoch',route=>route.fulfill({contentType:'text/html',body:PAGE}));
 await page.goto('/audio-epoch');await page.waitForFunction(()=>typeof renderRevision==='function');
 const result=await page.evaluate(({first,next})=>renderRevision(first,next,true),{first:wave(16384,128),next:wave(8192,128)});
 expect(result.after.active).toEqual([]);
 expect(result.left.slice(128).every(value=>value===0)).toBe(true);
 expect(result.right.slice(128).every(value=>value===0)).toBe(true);
});

const PHONE = `<!doctype html><base href='/client/'><script type='module'>
import {createPrivateAudio} from '/client/gui/private-audio.js';
window.startPhone=async()=>{
 const context=new OfflineAudioContext(2,512,24000);
 Object.defineProperty(context,'state',{get:()=> 'running'});context.close=async()=>{};
 const audio=createPrivateAudio({contextFactory:()=>context});await audio.ready;
 while(!audio.state().ready.includes('refused'))await new Promise(resolve=>setTimeout(resolve,0));
 const refuse=correlation=>{
  const event={actionId:'helm.test',correlation,lifecycleTransition:true};
  audio.action({...event,state:'Pressed'});audio.action({...event,state:'Refused'});
 };
 refuse('before');
 const first=context.suspend(128/24000),second=context.suspend(256/24000),rendering=context.startRendering();
 window.phone={audio,context,refuse,first,second,rendering};await first;
 window.phone.paused=true;
};</script>`;

test('a native-served phone retires cached feedback on its advertised revision without replay', {tag:'@core'},async({page})=>{
 let revision='1',sample=8192,probes=0,assetRequests=0;
 await page.route('**/audio-phone',route=>route.fulfill({contentType:'text/html',body:PHONE}));
 await page.route('**/host/asset-revision.json',route=>{probes++;return route.fulfill({contentType:'application/json',body:JSON.stringify({
  capability:'phoenix-native-asset-revision',version:1,revision,
 })});});
 await page.route('**/assets/sounds/*',route=>{assetRequests++;return route.fulfill({contentType:'audio/wav',body:Buffer.from(wave(sample))});});
 await page.goto('/audio-phone');await page.waitForFunction(()=>typeof startPhone==='function');
 await page.evaluate(()=>startPhone());
 const before=assetRequests;revision='2';sample=16384;
 await expect.poll(()=>probes).toBeGreaterThan(1);
 await expect.poll(()=>assetRequests).toBeGreaterThan(before);
 await page.waitForFunction(()=>phone.audio.state().ready.includes('refused'));
 expect(await page.evaluate(()=>phone.audio.state().active)).toEqual([]);
 await page.evaluate(async()=>{await phone.context.resume();await phone.second;phone.refuse('after');await phone.context.resume();});
 const result=await page.evaluate(async()=>{const output=await phone.rendering;phone.audio.dispose();return Array.from(output.getChannelData(0));});
 for(const value of result.slice(0,128))expect(value).toBeCloseTo(.12*8192/32767,5);
 expect(result.slice(128,256).every(value=>value===0)).toBe(true);
 for(const value of result.slice(256))expect(value).toBeCloseTo(.12*16384/32767,5);
});
