// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createHostAudio } from '../../gui/host-audio.js';
import { createPrivateAudio } from '../../gui/private-audio.js';
import privateManifest from '../../assets/audio/private-feedback.json';
import { renderAudioSettingsPanel } from '../../gui/audio-settings-panel.js';
import { createAudioLiveEquivalents } from '../../gui/audio-live-equivalents.js';
import { t } from '../../gui/strings.js';
import { AUDIO_CONFIG, FakeAudioContext, audioFetch, memoryStorage, settleAudio } from './audio-fixtures.js';

afterEach(() => { document.body.replaceChildren(); vi.useRealTimers(); });
const row = id => document.querySelector(`[data-audio-bus="${id}"]`);
const input = id => row(id).querySelector('input');
const mute = id => row(id).querySelector('button');
function mount() {
  const context = new FakeAudioContext();
  const audio = createHostAudio({ doc: document, storage: memoryStorage(),
    contextFactory: () => context, fetchAudio: audioFetch() });
  const dispose = renderAudioSettingsPanel(document, document.body, audio);
  return { audio, context, dispose };
}

describe('complete Audio controls', () => {
  it('private controls identify the real Interface test and preserve focus while changing portable preferences', async () => {
    const manifest = privateManifest;
    const context = new FakeAudioContext(); let saved;
    const audio = createPrivateAudio({manifest, contextFactory:()=>context, fetchAudio:audioFetch(),
      save:value=>{saved=value;return {status:'saved'};}});
    const dispose = renderAudioSettingsPanel(document, document.body, audio);
    await audio.ready; await settleAudio();
    expect([...document.querySelectorAll('[data-audio-bus]')].map(el=>el.dataset.audioBus)).toEqual(['master','alerts','interface']);
    expect(document.querySelector('[data-audio-test]').textContent).toBe(t('settings.audio.private_test'));
    expect(document.body.textContent).toContain(t('settings.audio.private_test_hint'));
    const slider = input('interface'); slider.focus(); slider.value='0.2'; slider.dispatchEvent(new Event('input'));
    expect(document.activeElement).toBe(slider); expect(saved.mix.interface.level).toBe(0.2);
    const applied = document.querySelector('[data-audio-cue="applied"]');
    expect(applied.checked).toBe(false); applied.click(); expect(saved.cues.applied).toBe(true);
    expect(document.querySelector('.audio-storage-status').textContent).toBe(t('settings.audio.private_saved'));
    dispose(); audio.dispose();
  });
  it('a private GM surface cannot reset the Viewscreen endpoint mix', () => {
    const storage = memoryStorage();
    const room = createHostAudio({ storage, contextFactory: () => null });
    room.setMasterVolume(0.2); room.dispose();
    const saved = new Map(storage.values);
    const gm = createHostAudio({ storage, isRoom: () => false });
    const dispose = renderAudioSettingsPanel(document, document.body, gm);
    expect([...document.querySelectorAll('input, button')].every(control => control.disabled)).toBe(true);
    document.querySelector('[data-audio-reset]').click();
    expect(storage.values).toEqual(saved);
    expect(gm.getMasterVolume()).toBe(0.2);
    dispose(); gm.dispose();
  });

  it('explains unavailable categories and updates controls in place when config arrives', async () => {
    const { audio, dispose } = mount();
    expect(input('music').disabled).toBe(false);
    expect(input('effects').disabled).toBe(true);
    expect(row('interface').textContent).toContain(t('settings.audio.no_interface'));
    const master = input('master'); master.focus();
    audio.audioConfig(JSON.stringify(AUDIO_CONFIG));
    await settleAudio();
    expect(input('master')).toBe(master);
    expect(document.activeElement).toBe(master);
    expect(input('effects').disabled).toBe(false);
    expect(input('alerts').disabled).toBe(false);
    expect(master.getAttribute('aria-label')).toBe(t('settings.audio.master'));
    master.value = '0.37'; master.dispatchEvent(new Event('input', { bubbles: true }));
    expect(audio.getMasterVolume()).toBe(0.37);
    expect(master.getAttribute('aria-valuetext')).toBe('37%');
    mute('master').click();
    expect(mute('master').getAttribute('aria-pressed')).toBe('true');
    expect(document.querySelector('.audio-output-status').textContent).toContain(t('settings.audio.master_silent'));
    mute('master').click();
    expect(input('master').value).toBe('0.37');
    document.querySelector('[data-audio-reset]').click();
    expect(input('master').value).toBe('1');
    dispose(); audio.dispose();
  });

  it('deliberate controls reach output and clean up while storage failure is legible', async () => {
    const { audio, context, dispose } = mount();
    await settleAudio();
    document.querySelector('[data-audio-test]').click();
    await settleAudio();
    expect(context.sample()).toBeGreaterThan(0);
    expect(document.querySelector('.audio-output-status').textContent).toContain(t('settings.audio.test_playing'));
    mute('master').click(); expect(context.sample()).toBe(0);
    context.advance(2); expect(audio.state().test).toBe('idle');
    dispose(); audio.dispose();
    const unavailable = createHostAudio({ contextFactory: () => null });
    const remove = renderAudioSettingsPanel(document, document.body, unavailable);
    expect(document.querySelector('.audio-storage-status').textContent).toBe(t('settings.audio.storage_unavailable'));
    expect(document.querySelector('.audio-output-status').textContent).toBe(t('settings.audio.output_unavailable'));
    expect(document.querySelector('[data-audio-test]').disabled).toBe(true);
    remove(); unavailable.dispose();
  });
});

it('equivalents show only current cues with direction, coalesce combat, and clear without any history', () => {
  vi.useFakeTimers();
  const view = createAudioLiveEquivalents(document);
  view.update({ kind: 'beam', active: true });
  view.update({ kind: 'blaster', x: 10, y: 0, z: 0 });
  expect(document.body.textContent).toContain(t('audio.cue.blaster', { bearing: 90, elevation: 0 }));
  view.update({ kind: 'blaster', x: 0, y: 0, z: -10 });
  expect(document.querySelectorAll('[data-audio-equivalent="blaster"]')).toHaveLength(1);
  expect(document.body.textContent).not.toContain('90°');
  view.update({ kind: 'impact' });
  vi.advanceTimersByTime(2000);
  expect(document.querySelectorAll('[data-audio-equivalent]')).toHaveLength(1);
  view.update({ kind: 'beam', active: false });
  expect(document.body.textContent).toBe('');
  view.update({ kind: 'impact' }); view.update({ kind: 'clear' });
  expect(document.body.textContent).toBe('');
  view.dispose();
});
