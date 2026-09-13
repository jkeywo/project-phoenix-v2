// @vitest-environment jsdom
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createPrivateAudio } from '../../gui/private-audio.js';
import { normalizePrivateAudio } from '../../gui/private-audio-preferences.js';
import { createGmConfirmationProfile } from '../../gui/gm-confirmation.js';
import { renderAudioSettingsPanel } from '../../gui/audio-settings-panel.js';
import { t } from '../../gui/strings.js';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const boot = readFileSync(path.join(root,'src/native_host/audio/private_boot.js'),'utf8');
const manifest = JSON.parse(readFileSync(path.join(root,'assets/audio/private-feedback.json'),'utf8'));
const storageBoot = readFileSync(path.join(root,'src/native_host/panes/operator_storage.js'),'utf8');
const savedProfile = JSON.parse(readFileSync(path.join(root,'tests/fixtures/native-private-profile-save.json'),'utf8')).profile;
let ready;
beforeEach(() => {
  vi.useFakeTimers(); ready = true;
  window.PhoenixOperatorStorage = {isReady:()=>ready};
  new Function(boot)();
});
afterEach(() => { vi.useRealTimers(); delete window.PhoenixPrivateAudioProvider; delete window.PhoenixOperatorStorage; });
const publish = (generation=1,status='playing') => window.__phoenixPrivateAudioApply(JSON.stringify({
  generation,status,test:'idle',categories:['alerts','interface'],surface:'helm',outputs:['Headset'],detail:''
}));
const drain = () => { const value=window.__phoenixPrivateAudioDrain(); return value ? JSON.parse(value) : null; };

it('shared semantic owner sends one fresh local cue, never a supplied device or room asset', async () => {
  const audio = createPrivateAudio({root:window,manifest,read:()=>normalizePrivateAudio()});
  await audio.ready; publish(); drain();
  const action = state => audio.action({state,correlation:'own-action',actionId:'fixture-action',lifecycleTransition:true});
  action('Pressed'); action('Pending'); action('Refused');
  const record = drain();
  expect(record.cue.id).toBe('refused');
  expect(record).toEqual({type:'NativePrivateAudio',generation:1,cue:{id:'refused',at_ms:Date.now(),test:false}});
  expect(drain()).toBeNull();
  action('Refused'); expect(drain()).toBeNull();
  audio.dispose();
});

it('waits for native profile load and drops missed, muted and replaced-generation cues', async () => {
  let preferences = normalizePrivateAudio();
  const audio = createPrivateAudio({root:window,manifest,read:()=>preferences,save:next=>{preferences=next;return {status:'saved'};}});
  await audio.ready; publish(); ready=false;
  expect(audio.click()).toBe(false); expect(drain()).toBeNull();
  ready=true; audio.reload(); expect(drain().mix.master.level).toBe(1);
  vi.advanceTimersByTime(200); audio.click(); vi.advanceTimersByTime(251); expect(drain()).toBeNull();
  audio.click(); publish(2); const baseline=drain();expect(baseline.cue).toBeUndefined();expect(baseline.mix.master.level).toBe(1);
  vi.advanceTimersByTime(200); audio.click(); audio.setBus('master',{muted:true}); audio.setBus('master',{muted:false});
  expect(drain().cue).toBeUndefined();
  vi.advanceTimersByTime(200); audio.click(); audio.setActive(false); expect(drain().stop).toBe(true);
  audio.setActive(true); expect(drain()).toBeNull(); audio.dispose();
});

it('deliberate output tests respect private mute and unavailable assigned output', async () => {
  const audio = createPrivateAudio({root:window,manifest,read:()=>normalizePrivateAudio()});
  await audio.ready; publish(); drain();
  expect(await audio.testOutput()).toBe(true); expect(drain().cue).toMatchObject({id:'test',test:true});
  expect(await audio.testOutput()).toBe(true); vi.advanceTimersByTime(251); expect(drain()).toBeNull();
  expect(audio.state().test).toBe('idle');
  publish(2,'failed'); expect(await audio.testOutput()).toBe(false);
  expect(await audio.enable()).toBe(false); expect(drain().retry).toBe(true);
  publish(3); audio.setBus('interface',{muted:true}); drain();
  expect(await audio.testOutput()).toBe(false); expect(drain()).toBeNull(); audio.dispose();
});

it('resends current quiet mix after generation replacement and ignores an older status', async () => {
  const preferences=normalizePrivateAudio({mono:true,mix:{master:{level:0.14,muted:false}}});
  const audio=createPrivateAudio({root:window,manifest,read:()=>preferences});
  await audio.ready;publish(1);expect(drain().mix.master.level).toBe(0.14);
  // A host boundary may have discarded that in-flight old-generation record.
  publish(2);audio.click();
  const record=drain();expect(record.generation).toBe(2);expect(record.mix.master.level).toBe(0.14);expect(record.mono).toBe(true);expect(record.cue.id).toBe('clicks');
  publish(1,'failed');expect(audio.state().generation).toBe(2);expect(audio.state().status).toBe('playing');
  audio.dispose();
});

it('GM native profile reload preserves quiet audio and existing choices without writing defaults or carrying identity', async () => {
  new Function(storageBoot)();
  const requests=[];window.PhoenixInstallNativeOperatorStorage(json=>{requests.push(JSON.parse(json));return true;});
  const profile=createGmConfirmationProfile({storage:window.PhoenixOperatorStorage});
  const audio=createPrivateAudio({root:window,manifest,read:()=>profile.audio(),save:value=>profile.setAudio(value)});
  const loaded=()=>profile.reload();window.addEventListener('phoenix-operator-profile-loaded',loaded);
  const unsubscribe=profile.subscribe(()=>audio.reload());
  const removePanel=renderAudioSettingsPanel(document,document.body,audio);
  await audio.ready;publish();
  expect(audio.click()).toBe(false);expect(drain()).toBeNull();
  expect(requests.map(value=>value.operation)).toEqual(['load']);
  window.__phoenixOperatorReply({operation:'load',status:'ok',profile:savedProfile});
  expect(audio.state().mix.master).toEqual({level:0.23,muted:true});
  expect(profile.mode('effect.damage')).toBe('immediate');
  expect(requests).toHaveLength(1);expect(drain().mix.master.muted).toBe(true);
  expect(await audio.testOutput()).toBe(false);
  audio.setBus('interface',{level:0.31});
  const saved=JSON.parse(requests.at(-1).profile);
  expect(saved.audio.mix.master).toEqual({level:0.23,muted:true});
  expect(saved.gmConfirmations['effect.damage']).toBe('immediate');
  expect(audio.state().persistence).toBe('saved');
  window.__phoenixOperatorReply({operation:'save',status:'error',error:'Disk write denied'});
  expect(audio.state().persistence).toBe('unavailable');
  expect(document.querySelector('.audio-storage-status').textContent).toBe(t('settings.audio.storage_unavailable'));
  expect(audio.state().mix.interface.level).toBe(0.31);
  expect(JSON.stringify(saved)).not.toMatch(/sessionToken|output:|NativePrivateAudio/);
  window.__phoenixOperatorReload();expect(audio.click()).toBe(false);
  window.removeEventListener('phoenix-operator-profile-loaded',loaded);unsubscribe();removePanel();audio.dispose();
});
