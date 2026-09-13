import { AUDIO_BUSES, normalizeAudioMix, defaultAudioMix, clampAudioLevel, audioSoundGain } from './audio-mix.js';
import { createRoomAudioPreferences } from './audio-preferences.js';
import { createBrowserAudioProvider } from './browser-audio-provider.js';
import { validDuckingSpec } from './audio-ducking.js';

/** Viewscreen consumer of existing authored config, live cue and HUD channels.
 * The host owns gameplay/envelopes/geometry; this module owns local presentation.
 * It never submits commands, stores cues or replays a missed one-shot. */
export function createHostAudio({
  doc = globalThis.document, storage = null, providerFactory = createBrowserAudioProvider,
  contextFactory, fetchAudio, onEquivalent = () => {}, isRoom = () => true,
  duckingSpec = null, fetchDucking = () => globalThis.fetch('assets/audio/room-ducking.json').then(response => {
    if (!response.ok) throw new Error('Audio settings unavailable');
    return response.json();
  }),
} = {}) {
  const preferences = createRoomAudioPreferences(storage);
  let mix = preferences.read().mix;
  let mono = preferences.read().mono;
  let cfg = null;
  let cfgText = null;
  let running = false;
  let suspended = false;
  let pageActive = true;
  let generation = null;
  let menu = false;
  let menuRegistered = false;
  let previousHud = null;
  let previousLevel = null;
  let hud = null;
  const listeners = new Set();
  const roomIds = new Set();
  const channelIds = ['ambient', 'engine', 'phaser', 'forcefield', 'music', 'siren'];
  const provider = providerFactory({
    contextFactory: contextFactory || (() => {
      const view = doc?.defaultView || globalThis;
      const Ctor = view.AudioContext || view.webkitAudioContext;
      return Ctor ? new Ctor() : null;
    }),
    fetchAudio,
    onChange: () => { for (const listener of listeners) listener(); },
  });
  provider.setMix(mix);
  provider.setMono?.(mono);
  const duckingReady = Promise.resolve().then(() => duckingSpec || (isRoom() ? fetchDucking() : null))
    .then(spec => { duckingSpec = spec; provider.setDucking?.(isRoom() && preferences.read().ducking, spec); })
    .catch(() => {});

  function emit(cue) { if (isRoom()) onEquivalent(cue); }
  function ensureMenu() {
    if (!menuRegistered && isRoom()) {
      // Existing menu asset and authored level; no redesign of the soundtrack.
      provider.register('menu', { file: 'assets/sounds/exploration.mp3', category: 'music', volume: 0.5, loop: true });
      menuRegistered = true;
    }
  }
  // Available before a world is selected, including the deliberate output test.
  ensureMenu();

  function state() { return { ...provider.snapshot(), ...preferences.read(), room: isRoom(),
    monoAvailable: typeof provider.setMono === 'function', duckingAvailable: validDuckingSpec(duckingSpec) === true }; }
  function notify() { for (const listener of listeners) listener(); }
  function setBus(id, change) {
    if (!AUDIO_BUSES.includes(id)) return;
    mix = normalizeAudioMix({ ...mix, [id]: { ...mix[id], ...change } });
    preferences.save(mix);
    provider.setMix(mix);
  }
  function resetMix() {
    mix = defaultAudioMix();
    mono = false;
    preferences.save(mix, mono, false);
    provider.setMix(mix);
    provider.setMono?.(mono);
    provider.setDucking?.(false, duckingSpec);
  }
  function setMono(value) {
    if (!isRoom()) return;
    mono = value === true;
    preferences.save(mix, mono);
    provider.setMono?.(mono);
    notify();
  }
  function setDucking(value) {
    if (!isRoom()) return;
    preferences.save(mix, mono, value === true);
    provider.setDucking?.(value === true, duckingSpec);
    notify();
  }

  function audioConfig(json) {
    let next;
    try { next = JSON.parse(json); } catch (_) { return; }
    if (!next || typeof next !== 'object' || Array.isArray(next) || json === cfgText) return;
    for (const id of roomIds) provider.remove(id);
    roomIds.clear();
    cfg = next;
    cfgText = json;
    previousHud = null;
    previousLevel = null;
    hud = null;
    if (!isRoom()) { provider.stopAll(); notify(); return; }
    function add(id, spec, category, volume, loop = true, spatial = null) {
      if (!spec?.file) return;
      provider.register(id, { file: spec.file, category, volume, loop, spatial,
        important: id === 'siren' || id === 'computer_warning' || id === 'computer_critical' });
      roomIds.add(id);
    }
    add('ambient', next.ambient, 'ambience', next.ambient?.volume);
    add('engine', next.engine, 'ambience', next.engine?.idle_volume);
    add('phaser', next.phaser_loop, 'effects', 0);
    add('forcefield', next.forcefield, 'effects', 0);
    add('blaster', next.blaster, 'effects', next.blaster?.volume, false, next.blaster);
    const alert = next.red_alert;
    add('music', { file: alert?.music_file }, 'music', alert?.music_volume);
    add('siren', { file: alert?.siren_file }, 'alerts', alert?.siren_volume, false);
    for (const severity of ['info', 'advisory', 'warning', 'critical']) {
      const spec = next.computer_message?.[severity];
      add(`computer_${severity}`, spec, 'alerts', spec?.volume, false);
    }
    reconcile();
    notify();
  }

  function reconcile() {
    if (!isRoom()) { provider.stopAll(); return; }
    provider.loop('menu', menu && !suspended && pageActive);
    const live = running && !suspended && pageActive;
    provider.loop('ambient', live, cfg?.ambient?.volume);
    const engine = cfg?.engine;
    if (engine) provider.loop('engine', live,
      engine.idle_volume + (Number(hud?.engine_thrust) || 0) * engine.volume_at_full_thrust);
    provider.loop('forcefield', live, previousLevel ?? 0);
    provider.loop('phaser', live && !!hud?.phaser_firing, cfg?.phaser_loop?.volume);
    provider.loop('music', live && !!hud?.red_alert);
  }

  function startMenuMusic() {
    if (!isRoom()) { provider.stopAll(); return; }
    if (menu) return;
    ensureMenu();
    menu = true;
    reconcile();
    void provider.enable();
  }
  function stopMenuMusic() { menu = false; provider.loop('menu', false); }
  function startGameAudio() {
    running = true;
    menu = false;
    reconcile();
    if (isRoom()) void provider.enable();
  }

  /** Current-state boundary, called on Lobby and restore/reconnect. Config and
   * preferences survive; the next HUD seeds red-alert state without a siren. */
  function resetSession() {
    provider.stopAll();
    previousHud = null;
    previousLevel = null;
    hud = null;
    running = false;
    menu = false;
    emit({ kind: 'clear' });
  }

  function audioLifecycle(json) {
    let value;
    try { value = JSON.parse(json); } catch (_) { return; }
    if (!value || !Number.isSafeInteger(value.generation) || typeof value.running !== 'boolean'
      || typeof value.suspended !== 'boolean' || value.generation === generation) return;
    generation = value.generation;
    const currentMenu = menu;
    resetSession();
    running = value.running;
    suspended = value.suspended;
    menu = currentMenu && !running;
    if (!running) {
      for (const id of roomIds) provider.remove(id);
      roomIds.clear(); cfg = null; cfgText = null;
    }
    reconcile();
  }

  // BFCache retains this facade and the runtime's current generation. Hide
  // stops voices; returning reads only the current loop state, never old cues.
  function setPageActive(active) {
    pageActive = !!active;
    previousHud = null;
    if (!pageActive) {
      provider.stopAll();
      emit({ kind: 'clear' });
    } else {
      reconcile();
      emit({ kind: 'beam', active: running && !suspended && !!hud?.phaser_firing });
    }
  }

  function audioCue(json) {
    let cue;
    try { cue = JSON.parse(json); } catch (_) { return; }
    if (!running || suspended || !pageActive || !isRoom() || !cue) return;
    if (cue.kind === 'computer_message') {
      // The existing computer banner is the equivalent: text, severity, Station.
      provider.cue(`computer_${cue.severity}`);
    } else if (cue.kind === 'blaster' && ['x', 'y', 'z'].every(key => Number.isFinite(cue[key]))) {
      if (!cfg?.blaster?.file) return;
      emit({ kind: 'blaster', x: cue.x, y: cue.y, z: cue.z });
      provider.cue('blaster', cue);
    }
  }
  function audioLevel(value) {
    const level = clampAudioLevel(value);
    if (running && !suspended && pageActive && cfg?.forcefield && previousLevel != null && level > previousLevel) emit({ kind: 'impact' });
    previousLevel = level;
    provider.loop('forcefield', running && !suspended && pageActive && isRoom(), level);
  }
  function applyHudAudio(value) {
    if (!value || !isRoom()) return;
    hud = value;
    if (running && !suspended && pageActive && previousHud && value.red_alert && !previousHud.red_alert) provider.cue('siren');
    // Existing Red Alert frame/status and computer banner remain their equivalents.
    emit({ kind: 'beam', active: running && !suspended && pageActive && !!value.phaser_firing });
    previousHud = value;
    reconcile();
  }

  function debug() {
    const output = provider.snapshot();
    const active = new Set(output.active.map(voice => voice.id));
    const els = channelIds.filter(id => roomIds.has(id));
    const categories = { ambient: 'ambience', engine: 'ambience', phaser: 'effects', forcefield: 'effects', music: 'music', siren: 'alerts' };
    return {
      cfg, els, started: running && !!cfg,
      master: mix.master.level, mix: normalizeAudioMix(mix),
      authoredVolumes: Object.fromEntries(els.map(id => [id, output.levels[id]])),
      volumes: Object.fromEntries(els.map(id => [id, audioSoundGain(mix, categories[id], output.levels[id])])),
      paused: Object.fromEntries(els.map(id => [id, !active.has(id)])),
      musicPlaying: active.has('music'), phaserPlaying: active.has('phaser'),
      blasterReady: output.ready.includes('blaster'), output, outputPeak: provider.outputPeak(),
    };
  }
  return {
    audioConfig, audioCue, audioLevel, audioLifecycle, applyHudAudio, startGameAudio, startMenuMusic, stopMenuMusic,
    resetSession, setPageActive, state, setBus, setMono, resetMix, setDucking, duckingReady,
    enable: () => isRoom() ? provider.enable() : Promise.resolve(false),
    testOutput: () => { ensureMenu(); return isRoom() ? provider.testOutput('menu') : Promise.resolve(false); },
    getMasterVolume: () => mix.master.level,
    setMasterVolume: value => setBus('master', { level: clampAudioLevel(value) }),
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    dispose: () => { provider.dispose(); listeners.clear(); emit({ kind: 'clear' }); },
    debug,
  };
}
