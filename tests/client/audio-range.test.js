// @vitest-environment jsdom
import { expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { audioRangeModule } from '../../scripts/audio-range.mjs';
import { createNativeAudio } from '../../gui/native-audio.js';
import { renderAudioSettingsPanel } from '../../gui/audio-settings-panel.js';
import { normalizePrivateAudio } from '../../gui/private-audio-preferences.js';
import { createHostAudio } from '../../gui/host-audio.js';
import { createClientSemanticActionRegistry } from '../../gui/client-semantic-actions.js';
import { serializeOperatorProfile, loadOperatorProfile, saveOperatorProfile } from '../../gui/operator-profile.js';
import { t } from '../../gui/strings.js';
import { FakeAudioContext, memoryStorage } from './audio-fixtures.js';

it('uses a generated browser projection of the native authored dynamics specification', async () => {
  expect(await audioRangeModule(resolve('.'))).toBe(readFileSync('gui/audio-range-data.js', 'utf8'));
});

it('preserves mono, ducking, display and private choices while range stays local and defaults off', () => {
  expect(normalizePrivateAudio().reducedRange).toBe(false);
  expect(normalizePrivateAudio({reducedRange:'true'}).reducedRange).toBe(false);
  const storage=memoryStorage(), audio=createHostAudio({storage,contextFactory:()=>new FakeAudioContext(),
    fetchAudio:async()=>({arrayBuffer:async()=>new ArrayBuffer(4)})});
  audio.setMono(true);audio.setDucking(true);audio.setReducedRange(true);audio.setMasterVolume(0.4);
  const record=JSON.parse(storage.getItem('phoenix-viewscreen-presentation-v1'));
  expect(record.audio).toMatchObject({mono:true,ducking:true,reducedRange:true,mix:{master:{level:0.4}}});
  // This deliberately limited endpoint lacks a compressor. The saved request
  // remains visible but is never reported as supported processing.
  expect(audio.state().reducedRangeAvailable).toBe(false);
  audio.resetMix();expect(audio.state()).toMatchObject({mono:false,ducking:false,reducedRange:false});
  const registry=createClientSemanticActionRegistry();
  const profile=loadOperatorProfile(storage,{registry}).profile;
  profile.audio=normalizePrivateAudio({mono:true,reducedRange:true,cues:{applied:true}});
  saveOperatorProfile(storage,profile);
  const saved=JSON.parse(serializeOperatorProfile(loadOperatorProfile(storage,{registry}).profile));
  expect(saved.audio).toMatchObject({mono:true,reducedRange:true,cues:{applied:true}});
  expect(saved.audio).not.toHaveProperty('output');expect(saved.audio).not.toHaveProperty('history');
  audio.dispose();
});

it('sends the typed native range option from a stable accessible control and states unsupported output', () => {
  const sent=[],win={}, audio=createNativeAudio({win,send:record=>sent.push(record)});
  const dispose=renderAudioSettingsPanel(document,document.body,audio);
  const control=document.querySelector('[data-audio-range]');
  expect(control.disabled).toBe(true);
  expect(document.body.textContent).toContain(t('settings.audio.reduced_range_unavailable'));
  win.__phoenixNativeAudioApply({room:true,categories:['music'],mix:{},reducedRange:false});
  control.focus();control.click();
  expect(sent.at(-1)).toEqual({kind:'set_audio_reduced_range',enabled:true});
  win.__phoenixNativeAudioApply({room:true,categories:['music'],mix:{},reducedRange:true});
  expect(control.checked).toBe(true);expect(document.activeElement).toBe(control);
  expect(document.querySelectorAll('[data-audio-range]')).toHaveLength(1);
  dispose();
});
