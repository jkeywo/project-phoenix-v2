import { AUDIO_BUSES, normalizeAudioMix, defaultAudioMix } from './audio-mix.js';
import { t } from './strings.js';

/** A control/observation adapter only: no AudioContext, HTMLAudioElement,
 * decoder, localStorage, game authority or private output fallback. */
export function createNativeAudio({ win = globalThis, send = () => {} } = {}) {
  const listeners = new Set();
  let current = { room: false, supportsDucking: true, ducking: false, mix: defaultAudioMix(), categories: [], mono: false, monoAvailable: false,
    status: 'unavailable', test: 'idle', persistence: 'unavailable', reducedRange: false, reducedRangeAvailable: false,
    hardware_persistence: 'unavailable', output: null, devices: [], detail: '', asset_failures: [] };
  function apply(json) {
    let value;
    try { value = typeof json === 'string' ? JSON.parse(json) : json; } catch (_) { return; }
    if (!value || value.room !== true || !Array.isArray(value.categories)) return;
    current = { ...current, ...value, mix: normalizeAudioMix(value.mix),
      mono: value.mono === true, monoAvailable: typeof value.mono === 'boolean',
      reducedRange: value.reducedRange === true, reducedRangeAvailable: typeof value.reducedRange === 'boolean' };
    for (const listener of listeners) listener();
  }
  win.__phoenixNativeAudioApply = apply;
  apply(win.__phoenixNativeAudioState);
  send({ kind: 'observe_audio' });
  const request = record => { if (current.room) send(record); };
  return {
    state: () => current,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    setBus(bus, change) {
      if (!AUDIO_BUSES.includes(bus) || (bus !== 'master' && !current.categories.includes(bus))) return;
      const value = normalizeAudioMix({ ...current.mix, [bus]: { ...current.mix[bus], ...change } })[bus];
      request({ kind: 'set_audio_bus', bus, level_percent: Math.round(value.level * 100), muted: value.muted });
    },
    resetMix: () => request({ kind: 'reset_audio_mix' }),
    setMono: enabled => request({ kind: 'set_audio_mono', enabled: enabled === true }),
    setDucking: enabled => request({ kind: 'set_audio_ducking', enabled: enabled === true }),
    setReducedRange: enabled => request({ kind: 'set_audio_reduced_range', enabled: enabled === true }),
    enable: () => request({ kind: 'retry_audio_output' }),
    testOutput: () => request({ kind: 'test_audio_output' }),
    selectOutput: output => request({ kind: 'select_audio_output', output: output || null }),
  };
}

/** Stable option nodes while status changes, so live playback cannot steal an
 * operator's selection/focus. Missing selection remains named until changed. */
export function renderNativeAudioOutput(doc, parent, audio) {
  const section = doc.createElement('section'); section.className = 'audio-settings';
  const label = doc.createElement('label'); label.textContent = t('settings.audio.native_output');
  const select = doc.createElement('select'); select.setAttribute('aria-label', label.textContent);
  const status = doc.createElement('p'); status.setAttribute('role', 'status');
  const storage = doc.createElement('p');
  const hint = doc.createElement('p'); hint.textContent = t('settings.audio.native_output_hint');
  label.append(select); section.append(label, status, storage, hint); parent.append(section);
  select.addEventListener('change', () => audio.selectOutput(select.value));
  let signature = null;
  function paint() {
    const state = audio.state();
    const choices = [{ id: '', label: t('settings.audio.system_default'), available: true }, ...(state.devices || [])];
    if (state.output && !choices.some(choice => choice.id === state.output)) {
      choices.push({ id: state.output, label: state.output, available: false });
    }
    const next = JSON.stringify(choices);
    if (next !== signature) {
      signature = next; select.replaceChildren();
      for (const choice of choices) {
        const option = doc.createElement('option'); option.value = choice.id;
        option.textContent = choice.available ? choice.label : t('settings.audio.output_unusable', { output: choice.label });
        option.disabled = !choice.available; select.append(option);
      }
    }
    select.value = state.output || ''; select.disabled = !state.room;
    const selected = choices.find(choice => choice.id === select.value);
    status.textContent = [selected?.label, state.detail ? t(state.detail) : '', ...(state.asset_failures || [])].filter(Boolean).join(' · ');
    storage.textContent = t(`settings.audio.hardware_${state.hardware_persistence || 'unavailable'}`)
      + (state.profile_override ? ' ' + t('settings.audio.hardware_profile') : '');
  }
  const unsubscribe = audio.subscribe(paint); paint();
  return () => { unsubscribe(); section.remove(); };
}
