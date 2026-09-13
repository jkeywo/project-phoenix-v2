import { AUDIO_BUSES } from './audio-mix.js';
import { t } from './strings.js';

/** A complete local mixer UI over the provider contract, reused by future
 * endpoints. No authority or device assignment is inferred from a DOM pane. */
export function renderAudioSettingsPanel(doc, parent, audio) {
  const panel = doc.createElement('section');
  panel.className = 'audio-settings';
  const rows = new Map();
  const text = (tag, id) => {
    const element = doc.createElement(tag);
    element.textContent = t(id);
    return element;
  };
  const privateSurface = audio?.state().private === true;
  panel.append(text('p', privateSurface ? 'settings.audio.private_scope' : 'settings.audio.scope'));
  for (const id of privateSurface ? audio.state().buses : AUDIO_BUSES) {
    const row = doc.createElement('fieldset');
    row.className = 'audio-settings-bus';
    row.dataset.audioBus = id;
    const title = text('legend', `settings.audio.${id}`);
    const slider = doc.createElement('input');
    slider.type = 'range';
    slider.min = '0'; slider.max = '1'; slider.step = '0.01';
    slider.className = 'server-settings-slider';
    slider.setAttribute('aria-label', t(`settings.audio.${id}`));
    const readout = doc.createElement('output');
    readout.className = 'server-settings-readout';
    const mute = text('button', 'settings.audio.mute');
    mute.type = 'button';
    mute.setAttribute('aria-label', t('settings.audio.mute_bus', { bus: t(`settings.audio.${id}`) }));
    const hint = doc.createElement('p');
    hint.className = 'server-settings-hint';
    const line = doc.createElement('div');
    line.className = 'audio-settings-controls';
    line.append(slider, readout, mute);
    row.append(title, line, hint);
    panel.append(row);
    slider.addEventListener('input', () => audio?.setBus(id, { level: Number(slider.value) }));
    mute.addEventListener('click', () => audio?.setBus(id, { muted: !audio.state().mix[id].muted }));
    rows.set(id, { row, slider, readout, mute, hint });
  }
  const status = doc.createElement('p');
  status.className = 'audio-output-status';
  status.setAttribute('role', 'status');
  const storageStatus = doc.createElement('p');
  storageStatus.className = 'audio-storage-status';
  const enable = text('button', 'settings.audio.enable');
  enable.type = 'button'; enable.dataset.audioEnable = '';
  enable.addEventListener('click', () => { void audio?.enable(); });
  const test = text('button', privateSurface ? 'settings.audio.private_test' : 'settings.audio.test');
  test.type = 'button'; test.dataset.audioTest = '';
  test.addEventListener('click', () => { void audio?.testOutput(); });
  const reset = text('button', 'settings.audio.reset');
  reset.type = 'button'; reset.dataset.audioReset = '';
  reset.addEventListener('click', () => audio?.resetMix());
  const actions = doc.createElement('div');
  actions.className = 'audio-settings-actions';
  actions.append(enable, test, reset);
  panel.append(status, storageStatus, actions, text('p', privateSurface
    ? 'settings.audio.private_test_hint' : 'settings.audio.test_hint'));
  const cueControls = new Map();
  if (privateSurface) {
    const group = doc.createElement('fieldset');
    group.append(text('legend', 'settings.audio.private_cues'));
    for (const id of Object.keys(audio.state().cues)) {
      const label = doc.createElement('label');
      const input = doc.createElement('input');
      input.type = 'checkbox'; input.dataset.audioCue = id;
      input.addEventListener('change', () => audio.setCue(id, input.checked));
      label.append(input, text('span', `settings.audio.cue_${id}`));
      group.append(label); cueControls.set(id, input);
    }
    panel.append(group);
  }
  parent.append(panel);

  function paint() {
    const state = audio?.state();
    for (const [id, controls] of rows) {
      const bus = state?.mix[id] || { level: 1, muted: false };
      const available = !!(state?.room || state?.private) && (id === 'master' || state.categories.includes(id));
      controls.slider.value = String(bus.level);
      controls.slider.disabled = !available;
      controls.mute.disabled = !available;
      controls.mute.setAttribute('aria-pressed', String(bus.muted));
      controls.readout.textContent = t('settings.master_volume_value', { value: Math.round(bus.level * 100) });
      controls.slider.setAttribute('aria-valuetext', controls.readout.textContent);
      controls.hint.textContent = available ? '' : t(id === 'interface'
        ? 'settings.audio.no_interface' : 'settings.audio.unconfigured');
      controls.hint.hidden = available;
    }
    const output = state?.status || 'unavailable';
    const lines = [t(`settings.audio.output_${output}`)];
    if (state?.native && state?.private) {
      lines.push(t('settings.audio.private_assignment', {
        surface: state.surface || '', outputs: (state.outputs || []).join(', ') || t('settings.audio.private_no_output'),
      }));
      if (state.detail) lines.push(state.detail.startsWith('settings.audio.') ? t(state.detail) : state.detail);
    }
    if (state?.mix.master.muted || state?.mix.master.level === 0) lines.push(t('settings.audio.master_silent'));
    if (state?.test === 'playing') lines.push(t('settings.audio.test_playing'));
    const testBus = state?.mix[state?.testBus || 'music'];
    if (testBus?.muted || testBus?.level === 0) lines.push(t(privateSurface
      ? 'settings.audio.private_test_silent' : 'settings.audio.test_silent'));
    if (state && !state.room && !state.private) lines.push(t('settings.audio.private_surface'));
    status.textContent = lines.join(' ');
    storageStatus.textContent = t(privateSurface && state?.persistence === 'saved'
      ? 'settings.audio.private_saved' : `settings.audio.storage_${state?.persistence || 'unavailable'}`);
    enable.disabled = !(state?.room || state?.private) || output === 'unavailable';
    test.disabled = !(state?.room || state?.private) || output === 'unavailable' || state?.test === 'loading';
    reset.disabled = !(state?.room || state?.private);
    for (const [id, input] of cueControls) input.checked = state.cues[id];
  }
  const unsubscribe = audio?.subscribe(paint);
  paint();
  return () => { unsubscribe?.(); panel.remove(); };
}
