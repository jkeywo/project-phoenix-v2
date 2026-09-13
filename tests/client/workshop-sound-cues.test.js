// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { URL as NodeURL } from 'node:url';
import { stringify } from 'smol-toml';
import { mountWorkshopSoundCues } from '../../editor/workshop-sound-cues.js';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { WORKSHOP_MANIFEST, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT } from '../fixtures/workshop-pack.js';
import { FakeAudioContext, settleAudio } from './audio-fixtures.js';
import { SOUND_CUES_PATH } from '../../gui/sound-cues.js';
const catalog=JSON.parse(readFileSync(new NodeURL('../../assets/audio/sound-cues.json',import.meta.url),'utf8'));
const manifest=JSON.parse(readFileSync(new NodeURL('../../assets/audio/private-feedback.json',import.meta.url),'utf8'));
afterEach(()=>{vi.unstubAllGlobals();vi.restoreAllMocks();document.body.replaceChildren();});
it('checks the editable catalog before structural export while preserving source bytes',()=>{
  const source='# Preserve this authored comment\r\n'+stringify(catalog).replaceAll('\n','\r\n');
  const document=new WorkshopDocument(createStoreZip([{path:'scenarios.toml',text:WORKSHOP_MANIFEST},
    {path:WORKSHOP_WORLD,text:WORKSHOP_WORLD_TEXT},{path:SOUND_CUES_PATH,text:source}]));
  expect(document.check().ok).toBe(true);document.edit(SOUND_CUES_PATH,source.replace('urgency = "warning"','urgency = "invented"'));
  expect(document.check().ok).toBe(false);document.undo();expect(document.read(SOUND_CUES_PATH)).toBe(source);
});
it('uses current source bytes, stops on draft revision and defers catalog refresh through exclusive Test',async()=>{
  const context=new FakeAudioContext({state:'running'});vi.stubGlobal('AudioContext',function(){return context;});
  vi.spyOn(window,'fetch').mockImplementation(async file=>({ok:true,json:async()=>file.includes('sound-cues')?catalog:manifest,arrayBuffer:async()=>new Float32Array([0.5]).buffer}));
  let source=stringify(catalog),samples=new Float32Array([0.8]);
  let draft={sourceRevision:0,read:()=>source,paths:()=>['assets/sounds/Blaster.mp3'],isBinary:()=>true,bytes:()=>new Uint8Array(samples.buffer)};
  const view=mountWorkshopSoundCues({root:document.body,win:window,draft:()=>draft,readAudio:()=>null,saveAudio:()=>({status:'saved'})});
  await view.ready;await settleAudio();view.refresh();await settleAudio();
  const select=document.querySelector('[data-sound-cue]'),preview=document.querySelector('[data-sound-preview]');
  select.value='weapons';select.dispatchEvent(new Event('change'));preview.click();await settleAudio();expect(context.sample()).toBeCloseTo(0.8*0.9);
  samples=new Float32Array([0.2]);draft.sourceRevision++;view.refresh({held:true});expect(context.sample()).toBe(0);
  source=source.replace('label = "[Weapon discharge]"','label = "[Edited cue]"');draft.sourceRevision++;view.refresh({held:true});
  view.refresh({held:false});await settleAudio();expect(select.selectedOptions[0].textContent).toContain('Edited cue');
  preview.click();await settleAudio();expect(context.sample()).toBeCloseTo(0.2*0.9);
  draft={...draft};view.refresh();expect(context.sample()).toBe(0);view.dispose();expect(context.sample()).toBe(0);
});
it('resolves a dependency override before base delivery and never fetches undeclared preview media',async()=>{
  const context=new FakeAudioContext({state:'running'});vi.stubGlobal('AudioContext',function(){return context;});
  const fetch=vi.spyOn(window,'fetch').mockImplementation(async file=>({ok:true,json:async()=>file.includes('sound-cues')?catalog:manifest}));
  const resolver=vi.fn(async file=>file==='assets/sounds/Blaster.mp3'?new Uint8Array(new Float32Array([0.3]).buffer):null);
  const draft={sourceRevision:0,read:()=>stringify(catalog),paths:()=>[],isBinary:()=>false};
  const view=mountWorkshopSoundCues({root:document.body,win:window,draft:()=>draft,resolveAsset:resolver,readAudio:()=>null,saveAudio:()=>({status:'saved'})});
  await view.ready;await settleAudio();const select=document.querySelector('[data-sound-cue]');select.value='weapons';select.dispatchEvent(new Event('change'));
  document.querySelector('[data-sound-preview]').click();await settleAudio();expect(context.sample()).toBeCloseTo(0.3*0.9);
  expect(resolver).toHaveBeenCalledWith('assets/sounds/Blaster.mp3');expect(fetch.mock.calls.every(([file])=>!file.startsWith('assets/sounds/'))).toBe(true);
  view.dispose();
});
