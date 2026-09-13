import { createBrowserAudioProvider } from './browser-audio-provider.js';
import { normalizePrivateAudio, PRIVATE_AUDIO_BUSES, PRIVATE_AUDIO_CUES, AUDITION_AUDIO_BUSES } from './private-audio-preferences.js';
import { validateSoundDefinition } from './sound-cues.js';

export const PRIVATE_AUDIO_MANIFEST = 'assets/audio/private-feedback.json';
const STATES = { Pending: 'pending', Applied: 'applied', Refused: 'refused', TimedOut: 'timedOut' };
const silentProvider = () => ({
  register() {}, cue: () => false, setMix() {}, enable: async () => false,
  testOutput: async () => false, stopAll() {}, dispose() {},
  snapshot: () => ({ status: 'unavailable', test: 'idle', categories: [] }),
});

/** One private document's output. No room channels, transport, cue history or
 * device identities. Missed transitions are consumed before attempting output. */
export function createPrivateAudio({
  root = globalThis, read = () => null, save = () => ({ status: 'unavailable' }),
  providerFactory, fetchManifest = (...args) => root.fetch(...args),
  manifest, now = () => Date.now(), contextFactory, fetchAudio,
  isEnabled = () => true,
  requireNativeProvider = false,
  allowAudition = false,
} = {}) {
  let preferences = normalizePrivateAudio(read());
  let persistence = 'unavailable', active = true, disposed = false;
  let spec = null;
  const listeners = new Set(), records = new Map(), lastCue = new Map();
  const notify = () => { for (const listener of listeners) listener(); };
  const nativeStorageStatus = () => {
    if (!root.PhoenixOperatorStorage) return;
    persistence = root.PhoenixOperatorStorageStatus?.status === 'error' ? 'unavailable' : 'saved';
    notify();
  };
  root.addEventListener?.('phoenix-operator-storage-status', nativeStorageStatus);
  if (root.PhoenixOperatorStorage?.isReady?.()) nativeStorageStatus();
  // Embedded browser-shaped APIs are not a native output assignment. A7 must
  // inject its explicit private adapter before mounting this document owner.
  const native = requireNativeProvider || root.PhoenixOperatorCapabilities?.surface === 'native-pane';
  const factory = providerFactory || root.PhoenixPrivateAudioProvider
    || (native ? silentProvider : createBrowserAudioProvider);
  let provider;
  try { provider = factory({ contextFactory, fetchAudio, onChange: notify }); }
  catch (_) { provider = silentProvider(); }
  provider.setMix(preferences.mix);
  provider.setMono?.(preferences.mono);
  provider.setReducedRange?.(preferences.reducedRange);
  function configure(value) {
    if (disposed || value?.version !== 1 || !value.sounds) return false;
    const accepted = {};
    for (const id of [...Object.keys(PRIVATE_AUDIO_CUES), 'test']) {
      const sound = value.sounds[id];
      if (!sound || !['alerts', 'interface'].includes(sound.category)
          || typeof sound.file !== 'string' || !sound.file.startsWith('assets/sounds/')
          || !Number.isFinite(sound.volume) || sound.volume < 0 || sound.volume > 1) return false;
      accepted[id] = sound;
    }
    spec = { sounds: accepted, coalesce_ms: Number.isFinite(value.coalesce_ms)
      ? Math.max(0, Math.min(1000, value.coalesce_ms)) : 100 };
    for (const [id, sound] of Object.entries(accepted)) provider.register(id, { ...sound, loop: false });
    notify();
    return true;
  }
  const ready = Promise.resolve().then(() => manifest || fetchManifest(PRIVATE_AUDIO_MANIFEST).then(r => {
    if (r.ok === false) throw new Error('private-audio-manifest-unavailable');
    return r.json();
  })).then(configure).catch(() => { notify(); return false; });
  function cue(id) {
    const stamp = now(), previous = lastCue.get(id);
    lastCue.set(id, stamp);
    if (!active || disposed || !spec || !isEnabled() || !preferences.cues[id]
        || (previous !== undefined && stamp - previous < spec.coalesce_ms)) return false;
    try { return provider.cue(id); } catch (_) { return false; }
  }
  function action(value, { continuous = false, hold = false } = {}) {
    if (!value || value.presentationRestored || value.lifecycleTransition !== true
        || typeof value.correlation !== 'string' || !value.correlation || !value.actionId) return false;
    const key = value.correlation;
    if (value.cancelled) { records.delete(key); return false; }
    if (value.state === 'Pressed') {
      if (records.has(key)) return false;
      if (records.size >= 128) records.delete(records.keys().next().value);
      records.set(key, { actionId: value.actionId, state: 'Pressed', quiet: !!(continuous || hold) });
      return false;
    }
    const record = records.get(key);
    if (!record || record.actionId !== value.actionId || !['Pressed', 'Pending'].includes(record.state)
        || !STATES[value.state] || record.state === value.state) return false;
    record.state = value.state;
    if (record.quiet) return false;
    // A provisional Pressed can be cancelled by an unavailable adapter. The
    // actual Pending edge confirms a handled activation and owns its click.
    if (value.state === 'Pending') return cue(preferences.cues.pending ? 'pending' : 'clicks');
    return cue(STATES[value.state]);
  }
  function reload() { preferences = normalizePrivateAudio(read()); provider.setMix(preferences.mix);
    provider.setMono?.(preferences.mono); provider.setReducedRange?.(preferences.reducedRange); notify(); }
  function change(next) {
    preferences = normalizePrivateAudio(next);
    let result;
    try { result = save(preferences); } catch (_) { result = { status: 'unavailable' }; }
    persistence = result?.status === 'saved' ? 'saved' : 'unavailable';
    provider.setMix(preferences.mix); provider.setMono?.(preferences.mono);
    provider.setReducedRange?.(preferences.reducedRange); notify();
    return result;
  }
  function setActive(value) {
    active = value === true;
    if (!active) { records.clear(); provider.stopAll(); }
    notify();
  }
  const auditionAvailable = () => allowAudition && typeof provider.audition === 'function'
    && (!native || provider.snapshot().audition === true);
  const buses = () => allowAudition ? AUDITION_AUDIO_BUSES : PRIVATE_AUDIO_BUSES;
  return {
    ready, action, click: () => cue('clicks'), actionable: () => cue('actionable'), reload, setActive,
    reset() { records.clear(); provider.stopAll(); },
    debug: () => ({ outputPeak: provider.outputPeak?.() || 0 }),
    state: () => ({ ...provider.snapshot(), mix: preferences.mix, cues: preferences.cues,
      mono: preferences.mono, monoAvailable: typeof provider.setMono === 'function',
      reducedRange: preferences.reducedRange,
      persistence, private: true, room: false, buses: buses(), testBus: 'interface',
      auditionAvailable: auditionAvailable(), categories: auditionAvailable() ? AUDITION_AUDIO_BUSES.slice(1) : provider.snapshot().categories }),
    setBus: (id, value) => buses().includes(id) && change({ ...preferences,
      mix: { ...preferences.mix, [id]: { ...preferences.mix[id], ...value } } }),
    setCue: (id, value) => Object.hasOwn(PRIVATE_AUDIO_CUES, id) && change({ ...preferences,
      cues: { ...preferences.cues, [id]: value === true } }),
    setMono: value => change({ ...preferences, mono: value === true }),
    setReducedRange: value => change({ ...preferences, reducedRange: value === true }),
    audition: async (definition, assets) => !validateSoundDefinition(definition, assets)
      && active && auditionAvailable() && await provider.audition(definition, assets.find(asset=>asset.file===definition.file)),
    stopAudition: () => provider.stopAudition?.(),
    resetMix: () => change({ ...preferences, mono: false, reducedRange: false, mix: normalizePrivateAudio().mix }),
    enable: async () => { try { return active && await provider.enable(); } catch (_) { return false; } },
    testOutput: async () => { try { return active && !!spec && await provider.testOutput('test'); } catch (_) { return false; } },
    subscribe: fn => { listeners.add(fn); return () => listeners.delete(fn); },
    dispose() { disposed = true; records.clear(); lastCue.clear(); provider.dispose(); listeners.clear();
      root.removeEventListener?.('phoenix-operator-storage-status', nativeStorageStatus); },
  };
}

/** The parent owns sound through child reloads. The caller verifies the current
 * source window; this forwarder sends only feedback metadata, never state. */
export function forwardPrivateActionFeedback(root, value, definition) {
  try {
    if (root.parent && root.parent !== root && typeof root.parent.__privateActionFeedback === 'function') {
      root.parent.__privateActionFeedback(root, value, {
        continuous: !!definition?.continuous, hold: !!definition?.hold,
      });
    }
  } catch (_) { /* Unavailable private output cannot interrupt an action. */ }
}

/** Only the currently mounted private console may feed its parent owner. A
 * replaced iframe or another Station cannot settle or register audio here. */
export function privateFeedbackReceiver({ getAudio, currentSource }) {
  return (source, value, metadata) => {
    if (!source || currentSource() !== source) return;
    const audio = getAudio();
    if (value?.state === 'Pressed') void audio?.enable();
    audio?.action(value, metadata);
  };
}

export function attachPrivateAudioLifecycle(audio, win) {
  const doc = win.document;
  const visibility = () => audio.setActive(!doc.hidden);
  const hide = () => audio.setActive(false);
  const gesture = () => { void audio.enable(); };
  doc.addEventListener('visibilitychange', visibility);
  win.addEventListener('pagehide', hide); win.addEventListener('pageshow', visibility);
  doc.addEventListener('pointerdown', gesture, true); doc.addEventListener('keydown', gesture, true);
  visibility();
  return () => {
    doc.removeEventListener('visibilitychange', visibility);
    win.removeEventListener('pagehide', hide); win.removeEventListener('pageshow', visibility);
    doc.removeEventListener('pointerdown', gesture, true); doc.removeEventListener('keydown', gesture, true);
    audio.dispose();
  };
}
if (typeof window !== 'undefined') window.PrivateAudio = { createPrivateAudio, attachPrivateAudioLifecycle, privateFeedbackReceiver };
