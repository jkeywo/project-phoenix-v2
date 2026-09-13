// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { URL as NodeURL } from 'node:url';
import { createPrivateAudio } from '../../gui/private-audio.js';
import { mountSoundAudition } from '../../gui/sound-audition-panel.js';
import { normalizePrivateAudio } from '../../gui/private-audio-preferences.js';
import { FakeAudioContext, audioFetch, settleAudio } from './audio-fixtures.js';
const catalog=JSON.parse(readFileSync(new NodeURL('../../assets/audio/sound-cues.json',import.meta.url),'utf8'));
const manifest=JSON.parse(readFileSync(new NodeURL('../../assets/audio/private-feedback.json',import.meta.url),'utf8'));
const boot=readFileSync(new NodeURL('../../src/native_host/audio/private_boot.js',import.meta.url),'utf8');
const nativePreview=JSON.parse(readFileSync(new NodeURL('../fixtures/native-private-audition.json',import.meta.url),'utf8'));
const cue=id=>catalog.cues.find(cue=>cue.id===id);
afterEach(()=>{document.body.replaceChildren();vi.useRealTimers();delete window.PhoenixPrivateAudioProvider;delete window.PhoenixOperatorStorage;});
async function fixture(options={}) {
  const context=new FakeAudioContext({state:'running'});let preferences=normalizePrivateAudio();
  const audio=createPrivateAudio({manifest,root:window,allowAudition:true,contextFactory:()=>context,fetchAudio:audioFetch(),
    read:()=>preferences,save:next=>{preferences=next;return{status:'saved'};},...options});
  await audio.ready;await settleAudio();return {audio,context,preferences:()=>preferences};
}
it('auditions each declared category through the same production mix, bounded output, mono, mute and scoped reset',async()=>{
  const {audio,context,preferences}=await fixture();
  for(const definition of catalog.cues) {
    audio.setBus('master',{level:0.4});audio.setBus(definition.category,{level:0.5});
    expect(await audio.audition(definition,catalog.assets)).toBe(true);
    expect(context.sample()).toBeCloseTo(0.5*0.4*0.5*definition.volume);
    expect(audio.state().active).toHaveLength(1);
    expect(audio.state().active[0].category).toBe(definition.category);
    audio.setBus(definition.category,{muted:true});expect(context.sample()).toBe(0);
    context.advance(2);audio.setBus(definition.category,{muted:false});expect(context.sample()).toBe(0);
  }
  audio.setMono(true);audio.setReducedRange(true);
  expect(preferences().mono).toBe(true);expect(preferences().reducedRange).toBe(true);
  audio.setBus('music',{level:0.23});
  expect(preferences().reducedRange).toBe(true);expect(preferences().mono).toBe(true);
  audio.resetMix();expect(preferences().mix.music).toEqual({level:1,muted:false});
  expect(preferences().mono).toBe(false);expect(preferences().reducedRange).toBe(false);audio.dispose();
});
it('does not let stopped, replaced or interrupted asynchronous decode start an obsolete preview',async()=>{
  let resolve;const {audio,context}=await fixture({fetchAudio:async()=>({ok:true,arrayBuffer:()=>new Promise(r=>{resolve=r;})})});
  const first=audio.audition(cue('exploration'),catalog.assets);await settleAudio();audio.stopAudition();resolve(new Float32Array([0.5]).buffer);
  expect(await first).toBe(false);expect(context.sample()).toBe(0);
  const second=audio.audition(cue('ambient'),catalog.assets);await settleAudio();
  context.state='suspended';context.onstatechange();context.state='running';context.onstatechange();
  resolve(new Float32Array([0.5]).buffer);expect(await second).toBe(false);expect(context.sample()).toBe(0);audio.dispose();
});
it('keeps informative equivalents visible when muted, never dispatches live actions, and stops on selection, hold and close',async()=>{
  const {audio,context}=await fixture();const dispatch=vi.fn();window.__hostFireGmEvent=dispatch;
  const panel=mountSoundAudition({root:document.body,audio,load:async()=>catalog});await panel.ready;
  const select=document.querySelector('[data-sound-cue]'),preview=document.querySelector('[data-sound-preview]');
  select.value='weapons';select.dispatchEvent(new Event('change'));
  audio.setBus('master',{muted:true});preview.click();await settleAudio();
  expect(context.sample()).toBe(0);expect(document.querySelector('[data-sound-equivalent]').textContent).toContain('45');
  audio.setBus('master',{muted:false});preview.click();await settleAudio();expect(context.sample()).toBeGreaterThan(0);
  panel.setHeld(true);expect(context.sample()).toBe(0);expect(document.querySelector('.sound-audition').hidden).toBe(true);
  panel.setHeld(false);expect(context.sample()).toBe(0);expect(dispatch).not.toHaveBeenCalled();
  panel.dispose();audio.dispose();delete window.__hostFireGmEvent;
});
it('native preview requires actual assigned GM capability, acknowledges output, and discards boundary/replacement requests',async()=>{
  vi.useFakeTimers();window.PhoenixOperatorStorage={isReady:()=>true};new Function(boot)();
  const {audio}=await fixture({requireNativeProvider:true});
  const publish=(extra={})=>window.__phoenixPrivateAudioApply(JSON.stringify({generation:1,status:'playing',test:'idle',categories:['alerts','interface'],
    surface:'native-gm',outputs:['GM headphones'],audition:true,preview:'idle',preview_id:0,...extra}));
  const drain=()=>JSON.parse(window.__phoenixPrivateAudioDrain()||'null');
  expect(await audio.audition(cue('weapons'),catalog.assets)).toBe(false);
  publish();drain();const playing=audio.audition(cue('weapons'),catalog.assets);const sent=drain();
  expect(sent).toEqual({...nativePreview,generation:1,preview:{...nativePreview.preview,at_ms:Date.now()}});
  expect(sent.preview.definition).toEqual(cue('weapons'));expect(sent).not.toHaveProperty('outputs');
  publish({preview_id:sent.preview.id,preview:'playing'});expect(await playing).toBe(true);
  audio.stopAudition();expect(drain().stop_preview).toBe(true);
  const pending=audio.audition(cue('ambient'),catalog.assets);publish({generation:2});
  expect(await pending).toBe(false);expect(drain()).not.toHaveProperty('preview');
  publish({generation:2,audition:false});expect(await audio.audition(cue('ambient'),catalog.assets)).toBe(false);
  audio.dispose();
});
it('refuses unvalidated assets and informative definitions before calling either provider',async()=>{
  const {audio,context}=await fixture();const before=context.sources.length;
  expect(await audio.audition({...cue('weapons'),equivalent:null},catalog.assets)).toBe(false);
  expect(await audio.audition({...cue('interface-click'),file:'https://example.test/voice.ogg'},catalog.assets)).toBe(false);
  expect(context.sources).toHaveLength(before);audio.dispose();
});
