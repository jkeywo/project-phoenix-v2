// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createNativeAudio, renderNativeAudioOutput } from '../../gui/native-audio.js';
import { mountNativeSettings } from '../../gui/native-settings.js';
import { defaultAudioMix, normalizeAudioMix, audioSoundGain } from '../../gui/audio-mix.js';
import policies from '../fixtures/audio-gain-policy.json';
import { t } from '../../gui/strings.js';

afterEach(() => document.body.replaceChildren());
function setup() {
  const send = vi.fn(); const win = {};
  const audio = createNativeAudio({ win, send });
  const state = { room: true, mix: defaultAudioMix(), categories: ['music', 'ambience', 'alerts'],
    status: 'playing', test: 'idle', persistence: 'saved', hardware_persistence: 'saved',
    output: null, devices: [{ id: 'output:Speakers', label: 'Speakers', available: true }], detail: '', asset_failures: [] };
  const apply = (change = {}) => { Object.assign(state, change); win.__phoenixNativeAudioApply(JSON.stringify(state)); };
  apply(); return { audio, send, win, state, apply };
}
describe('native Viewscreen audio controls', () => {
  it('shares authored gain and mute policy fixtures with the native sample provider', () => {
    for (const policy of policies) {
      expect(audioSoundGain(normalizeAudioMix(policy.mix), policy.category, policy.authored)).toBeCloseTo(policy.expected);
    }
  });
  it('mount and reload observe the sole native owner; only explicit Test asks for sound', () => {
    const { audio, send, win } = setup();
    expect(send.mock.calls).toEqual([[{ kind: 'observe_audio' }]]);
    const shell = mountNativeSettings(document, { audio }); shell.open();
    expect(send).toHaveBeenCalledTimes(1);
    document.querySelector('[data-audio-test]').click();
    expect(send).toHaveBeenLastCalledWith({ kind: 'test_audio_output' });
    shell.close(); shell.open();
    expect(send).toHaveBeenCalledTimes(2);
    createNativeAudio({ win, send });
    expect(send).toHaveBeenLastCalledWith({ kind: 'observe_audio' });
  });
  it('Master and supported categories send bounded typed changes; unimplemented categories are explained', () => {
    const { audio, send, apply, state } = setup();
    const shell = mountNativeSettings(document, { audio }); shell.open();
    const master = document.querySelector('[data-audio-bus="master"]');
    const slider = master.querySelector('input'); slider.value = '0.27'; slider.dispatchEvent(new Event('input'));
    expect(send).toHaveBeenLastCalledWith({ kind: 'set_audio_bus', bus: 'master', level_percent: 27, muted: false });
    state.mix.master.level = 0.27; apply(); master.querySelector('button').click();
    expect(send).toHaveBeenLastCalledWith({ kind: 'set_audio_bus', bus: 'master', level_percent: 27, muted: true });
    for (const category of ['effects', 'interface']) {
      const row = document.querySelector(`[data-audio-bus="${category}"]`);
      expect(row.querySelector('input').disabled).toBe(true);
      expect(row.querySelector('p').textContent.length).toBeGreaterThan(0);
    }
    state.mix.master.muted = true; apply();
    expect(document.querySelector('.audio-output-status').textContent).toContain(t('settings.audio.master_silent'));
    expect(master.querySelector('button').getAttribute('aria-pressed')).toBe('true');
  });
  it('selected loss is named and stays selected, with stable focused options and deliberate recovery', () => {
    const { audio, send, apply } = setup();
    const dispose = renderNativeAudioOutput(document, document.body, audio);
    const select = document.querySelector('select'); select.value = 'output:Speakers'; select.dispatchEvent(new Event('change'));
    expect(send).toHaveBeenLastCalledWith({ kind: 'select_audio_output', output: 'output:Speakers' });
    apply({ output: 'output:Speakers' }); select.focus(); const option = select.options[1];
    apply({ test: 'playing' }); expect(document.activeElement).toBe(select); expect(select.options[1]).toBe(option);
    apply({ devices: [], detail: 'settings.audio.selected_missing', status: 'failed' });
    expect(select.value).toBe('output:Speakers');
    expect(document.querySelector('[role="status"]').textContent).toContain(t('settings.audio.selected_missing'));
    audio.enable(); expect(send).toHaveBeenLastCalledWith({ kind: 'retry_audio_output' });
    select.value = ''; select.dispatchEvent(new Event('change'));
    expect(send).toHaveBeenLastCalledWith({ kind: 'select_audio_output', output: null }); dispose();
  });
  it('an unconnected document has no room authority or persistence fallback', () => {
    const send = vi.fn(); const audio = createNativeAudio({ win: {}, send });
    const shell = mountNativeSettings(document, { audio }); shell.open();
    for (const element of document.querySelectorAll('.audio-settings input, .audio-settings button, .audio-settings select')) {
      expect(element.disabled).toBe(true);
    }
    audio.setBus('master', { level: 0 }); audio.testOutput(); audio.resetMix();
    expect(send.mock.calls).toEqual([[{ kind: 'observe_audio' }]]);
  });
});
