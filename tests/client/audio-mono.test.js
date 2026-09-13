// @vitest-environment jsdom
import { expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { createHostAudio } from '../../gui/host-audio.js';
import { createPrivateAudio } from '../../gui/private-audio.js';
import { createNativeAudio } from '../../gui/native-audio.js';
import { renderAudioSettingsPanel } from '../../gui/audio-settings-panel.js';
import { normalizePrivateAudio } from '../../gui/private-audio-preferences.js';
import { createClientSemanticActionRegistry } from '../../gui/client-semantic-actions.js';
import { serializeOperatorProfile, loadOperatorProfile, saveOperatorProfile } from '../../gui/operator-profile.js';
import { t } from '../../gui/strings.js';
import { AUDIO_CONFIG, FakeAudioContext, memoryStorage, settleAudio } from './audio-fixtures.js';
const manifest = JSON.parse(readFileSync('assets/audio/private-feedback.json', 'utf8'));
const fetchStereo = (left, right) => async () => ({ok:true,arrayBuffer:async()=>new Float32Array([left,right]).buffer});
const pair = (context, left, right) => {
  const actual = context.sampleChannels(); expect(actual[0]).toBeCloseTo(left); expect(actual[1]).toBeCloseTo(right);
};

it('converts an active room loop live without replacing it and preserves independent endpoint fields', async () => {
  const storage=memoryStorage({'phoenix-viewscreen-presentation-v1':JSON.stringify({audio:{version:1,ducking:true},presentation:{textScale:1.5}})});
  const context=new FakeAudioContext();
  const audio=createHostAudio({storage,contextFactory:()=>context,fetchAudio:fetchStereo(0.6,0.2)});
  audio.audioConfig(JSON.stringify({ambient:{file:'ambient.ogg',volume:0.5}}));audio.startGameAudio();await settleAudio();
  pair(context,0.3,0.1);expect(audio.state().mono).toBe(false);
  const voice=context.sources.find(source=>source.started);context.advance(2);
  audio.setMono(true);pair(context,0.2,0.2);expect(context.sources.find(source=>source.started)).toBe(voice);expect(voice.startAt).toBe(0);
  audio.setBus('ambience',{level:0.5});audio.setMasterVolume(0.4);pair(context,0.04,0.04);
  audio.setBus('ambience',{muted:true});pair(context,0,0);
  audio.setBus('ambience',{muted:false});audio.setMono(false);pair(context,0.06,0.02);
  audio.setMono(true);
  const saved=JSON.parse(storage.getItem('phoenix-viewscreen-presentation-v1'));expect(saved.audio).toMatchObject({mono:true,ducking:true});
  const next=createHostAudio({storage,contextFactory:()=>new FakeAudioContext(),fetchAudio:fetchStereo(1,0)});
  expect(next.state().mono).toBe(true);next.resetMix();expect(next.state().mono).toBe(false);
  expect(JSON.parse(storage.getItem('phoenix-viewscreen-presentation-v1')).presentation.textScale).toBe(1.5);
  audio.dispose();next.dispose();
});

it('a right-only authored-shaped room cue reaches both ears after conversion with the same live direction', async () => {
  const context=new FakeAudioContext(), equivalents=[];
  const audio=createHostAudio({contextFactory:()=>context,fetchAudio:fetchStereo(0,0.8),onEquivalent:value=>equivalents.push(value)});
  audio.audioConfig(JSON.stringify({blaster:AUDIO_CONFIG.blaster}));audio.startGameAudio();await settleAudio();
  audio.setMono(true);audio.setBus('effects',{level:0.25});audio.setMasterVolume(0.4);
  const cue={kind:'blaster',x:30,y:4,z:-15};audio.audioCue(JSON.stringify(cue));pair(context,0.036,0.036);
  expect(equivalents).toContainEqual(cue);
  audio.setMasterVolume(0);pair(context,0,0);const count=audio.state().active.length;
  audio.audioCue(JSON.stringify(cue));expect(audio.state().active).toHaveLength(count);
  audio.dispose();
});

it('private native-portable mono reaches the actual browser provider while audio reset preserves cue choices', async () => {
  const context=new FakeAudioContext(), storage=memoryStorage();
  const registry=createClientSemanticActionRegistry();
  const load=()=>loadOperatorProfile(storage,{registry});
  const initial=load().profile;initial.audio=normalizePrivateAudio({mono:true,cues:{applied:true}});
  saveOperatorProfile(storage,initial);
  const audio=createPrivateAudio({root:window,manifest,contextFactory:()=>context,fetchAudio:fetchStereo(0.8,0),
    read:()=>load().profile.audio,save:audio=>saveOperatorProfile(storage,{...load().profile,audio})});
  await audio.ready;await settleAudio();await audio.enable();expect(audio.actionable()).toBe(true);pair(context,0.064,0.064);
  audio.setBus('alerts',{muted:true});pair(context,0,0);audio.setBus('master',{level:0});pair(context,0,0);
  audio.setMono(false);expect(JSON.parse(serializeOperatorProfile(load().profile)).audio.mono).toBe(false);
  audio.setMono(true);audio.resetMix();expect(audio.state().mono).toBe(false);expect(audio.state().cues.applied).toBe(true);
  audio.dispose();
});

it('shared native control keeps focus, sends only typed local mono, and states missing capability', () => {
  const sent=[],win={};const audio=createNativeAudio({win,send:value=>sent.push(value)});
  const dispose=renderAudioSettingsPanel(document,document.body,audio);const control=document.querySelector('[data-audio-mono]');
  expect(control.disabled).toBe(true);expect(document.body.textContent).toContain(t('settings.audio.mono_unavailable'));
  win.__phoenixNativeAudioApply({room:true,categories:['music'],mix:{},mono:false});control.focus();control.click();
  expect(sent.at(-1)).toEqual({kind:'set_audio_mono',enabled:true});
  win.__phoenixNativeAudioApply({room:true,categories:['music'],mix:{},mono:true});
  expect(document.activeElement).toBe(control);expect(control.checked).toBe(true);dispose();
});
