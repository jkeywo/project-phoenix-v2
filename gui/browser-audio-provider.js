import { AUDIO_CATEGORIES, normalizeAudioMix, audioBusGain, audioSoundGain, clampAudioLevel } from './audio-mix.js';
import { validDuckingSpec, nextDuck, duckGain, releaseDuck } from './audio-ducking.js';
import { createAudioRange } from './audio-range.js';

/** Production browser playback adapter. [ai] Every sample takes the same route:
 * decoded buffer -> authored gain (optional listener-relative panner) -> category
 * -> Master -> output. Live bus changes therefore also reach overlapping shots.
 * Desired loops are current state; one-shots are consumed immediately or dropped.
 * Nothing waiting for unlock, decode or restore is a one-shot backlog. */
export function createBrowserAudioProvider({
  contextFactory = () => {
    const Ctor = globalThis.AudioContext || globalThis.webkitAudioContext;
    return Ctor ? new Ctor() : null;
  },
  fetchAudio = (...args) => globalThis.fetch(...args),
  onChange = () => {},
} = {}) {
  let context = null;
  let master = null;
  let meter = null;
  let unavailable = false;
  let failed = false;
  let disposed = false;
  let testGeneration = 0;
  let testState = 'idle';
  let mix = normalizeAudioMix();
  let mono = false;
  let reducedRange = false;
  let range = null;
  const buses = new Map();
  const sounds = new Map();
  const voices = new Set();
  const cache = new Map();
  const beds = new Map();
  let ducking = false, duckSpec = null, duckWindow = null;
  function changed() { onChange(); }

  function scheduleDuck(window) {
    duckWindow = window;
    const now = context?.currentTime ?? 0;
    for (const node of beds.values()) {
      const gain = node.gain;
      gain.cancelScheduledValues(now);
      gain.setValueAtTime(duckGain(window, now, duckSpec), now);
      if (window) {
        if (window.attackEnd > now) gain.linearRampToValueAtTime(window.floor, window.attackEnd);
        gain.setValueAtTime(window.floor, window.hold);
        gain.linearRampToValueAtTime(1, window.end);
      }
    }
  }
  function setDucking(enabled, spec = duckSpec) {
    const wasEnabled = ducking;
    ducking = enabled === true && validDuckingSpec(spec);
    duckSpec = validDuckingSpec(spec) ? spec : null;
    if (!duckSpec) scheduleDuck(null);
    else if (wasEnabled && !ducking) scheduleDuck(releaseDuck(duckWindow, context?.currentTime ?? 0, duckSpec));
    changed();
  }
  function duck() {
    if (!ducking || context?.state !== 'running') return;
    const next = nextDuck(duckWindow, context.currentTime, duckSpec);
    if (next !== duckWindow) scheduleDuck(next);
  }

  function graph() {
    if (unavailable || disposed) return null;
    if (context) return context;
    try {
      context = contextFactory();
      if (!context) { unavailable = true; changed(); return null; }
      master = context.createGain();
      // Keep the endpoint stereo even when every active voice is mono, so
      // speaker up-mixing duplicates the allowed mono signal before Master.
      master.channelCount = 2;
      master.channelCountMode = 'explicit';
      master.channelInterpretation = 'speakers';
      meter = context.createAnalyser();
      meter.fftSize = 256;
      master.connect(meter).connect(context.destination);
      const rangeInput = context.createGain();
      range = createAudioRange(context, rangeInput, master);
      range.set(reducedRange);
      for (const id of AUDIO_CATEGORIES) {
        const gain = context.createGain();
        if (id === 'music' || id === 'ambience') {
          const bed = context.createGain();
          gain.connect(bed).connect(rangeInput);
          beds.set(id, bed);
        } else gain.connect(rangeInput);
        buses.set(id, gain);
      }
      context.onstatechange = () => {
        if (context.state === 'running') reconcileLoops();
        else {
          // A suspended context freezes sample time. Keeping a one-shot here
          // would replay its missed remainder after an unlock/device return.
          testGeneration++;
          testState = 'idle';
          for (const voice of [...voices]) stopVoice(voice);
          range?.reset();
          scheduleDuck(null);
          // Desired loops are current state and can be rederived on resume.
        }
        changed();
      };
      setMix(mix);
      return context;
    } catch (_) {
      unavailable = true;
      changed();
      return null;
    }
  }

  function setMix(value) {
    const next = normalizeAudioMix(value);
    const quieted = ['master', ...AUDIO_CATEGORIES].filter(id =>
      audioBusGain(mix, id) > 0 && audioBusGain(next, id) === 0);
    mix = next;
    if (master) master.gain.value = audioBusGain(mix, 'master');
    for (const [id, gain] of buses) gain.gain.value = audioBusGain(mix, id);
    if (quieted.length) {
      // The compressor delays a few milliseconds of already-mixed samples.
      // Rebuild it at the mute boundary so a quick unmute cannot expose them.
      range?.reset();
      testGeneration++;
      testState = 'idle';
      for (const voice of [...voices]) {
        if (!voice.source.loop && (quieted.includes('master') || quieted.includes(voice.sound.category))) stopVoice(voice);
      }
    }
    changed();
  }

  // Explicit one-channel speaker mixing averages L/R before the authored and
  // category gains; downstream speaker mixing sends that signal to both ears.
  function monoInput(gain) {
    gain.channelCount = mono ? 1 : 2;
    gain.channelCountMode = mono ? 'explicit' : 'max';
    gain.channelInterpretation = 'speakers';
  }
  function setMono(value) {
    mono = value === true;
    for (const voice of voices) monoInput(voice.gain);
    changed();
  }
  function setReducedRange(value) {
    reducedRange = value === true;
    range?.set(reducedRange);
    changed();
  }

  function bufferFor(file) {
    if (cache.has(file)) return cache.get(file);
    const pending = Promise.resolve().then(async () => {
      const response = await fetchAudio(file);
      if (response.ok === false) throw new Error('Audio asset unavailable');
      return context.decodeAudioData(await response.arrayBuffer());
    }).catch(() => {
      cache.delete(file);
      return null;
    });
    cache.set(file, pending);
    return pending;
  }

  function prepare(sound) {
    if (!graph()) return Promise.resolve(null);
    sound.loading = true;
    sound.failed = false;
    const pending = bufferFor(sound.file).then(buffer => {
      if (disposed || sounds.get(sound.id) !== sound) return null;
      sound.loading = false;
      sound.buffer = buffer;
      sound.failed = !buffer;
      if (buffer) reconcileLoop(sound);
      changed();
      return buffer;
    });
    sound.pending = pending;
    return pending;
  }

  function register(id, spec) {
    remove(id);
    if (!spec?.file || !AUDIO_CATEGORIES.includes(spec.category)) return;
    const sound = { ...spec, id, level: clampAudioLevel(spec.volume), wanted: false, buffer: null };
    sounds.set(id, sound);
    prepare(sound);
  }

  function cleanVoice(voice) {
    if (!voices.delete(voice)) return;
    voice.source.onended = null;
    for (const node of voice.nodes) { try { node.disconnect(); } catch (_) { /* already closed */ } }
    if (voice.sound.voice === voice) voice.sound.voice = null;
    if (voice.test) testState = 'idle';
    changed();
  }

  function stopVoice(voice) {
    try { voice.source.stop(); } catch (_) { /* already ended */ }
    cleanVoice(voice);
  }

  function play(sound, { position = null, duration = null, test = false } = {}) {
    if (!sound.buffer || context?.state !== 'running' || disposed) return false;
    let voice = null;
    try {
      const source = context.createBufferSource();
      source.buffer = sound.buffer;
      source.loop = !!sound.loop && !test;
      const gain = context.createGain();
      monoInput(gain);
      gain.gain.value = sound.level;
      const nodes = [source, gain];
      let tail = source;
      if (position && sound.spatial) {
        const pan = context.createPanner();
        const spec = sound.spatial;
        Object.assign(pan, {
          panningModel: spec.panning_model, distanceModel: spec.distance_model,
          refDistance: spec.ref_distance, maxDistance: spec.max_distance, rolloffFactor: spec.rolloff_factor,
        });
        if (pan.positionX) {
          pan.positionX.value = position.x;
          pan.positionY.value = position.y;
          pan.positionZ.value = position.z;
        } else pan.setPosition(position.x, position.y, position.z);
        tail = tail.connect(pan);
        nodes.push(pan);
      }
      tail.connect(gain).connect(buses.get(sound.category));
      voice = { source, gain, nodes, sound, test };
      voices.add(voice);
      source.onended = () => cleanVoice(voice);
      source.start();
      if (sound.important && sound.category === 'alerts' && !test) duck();
      if (duration != null) source.stop(context.currentTime + Math.min(duration, sound.buffer.duration));
      if (source.loop) sound.voice = voice;
      if (test) testState = 'playing';
      changed();
      return true;
    } catch (_) {
      if (voice) stopVoice(voice);
      failed = true; changed(); return false;
    }
  }

  function reconcileLoop(sound) {
    if (sound.loop && sound.wanted && !sound.voice) play(sound);
  }
  function reconcileLoops() { for (const sound of sounds.values()) reconcileLoop(sound); }

  function loop(id, wanted, volume) {
    const sound = sounds.get(id);
    if (!sound) return;
    sound.wanted = !!wanted;
    if (volume != null) {
      sound.level = clampAudioLevel(volume);
      if (sound.voice) sound.voice.gain.gain.value = sound.level;
    }
    if (!wanted && sound.voice) stopVoice(sound.voice);
    reconcileLoop(sound);
  }

  function cue(id, position) {
    const sound = sounds.get(id);
    // Muted, blocked and not-yet-decoded cues are missed, never queued.
    if (!sound || audioSoundGain(mix, sound.category, sound.level) === 0) return false;
    return play(sound, { position });
  }

  function remove(id) {
    const sound = sounds.get(id);
    if (!sound) return;
    for (const voice of [...voices]) if (voice.sound === sound) stopVoice(voice);
    sounds.delete(id);
    if (![...sounds.values()].some(other => other.file === sound.file)) cache.delete(sound.file);
  }

  function stopAll() {
    range?.reset();
    testGeneration++;
    testState = 'idle';
    for (const sound of sounds.values()) sound.wanted = false;
    for (const voice of [...voices]) stopVoice(voice);
    scheduleDuck(null);
    changed();
  }

  async function enable() {
    if (!graph()) return false;
    failed = false;
    try {
      if (context.state !== 'running') await context.resume();
      for (const sound of sounds.values()) if (sound.failed) prepare(sound);
      reconcileLoops();
      changed();
      return context.state === 'running';
    } catch (_) {
      // A gesture refusal and a broken device are distinct from volume/mute.
      if (context.state !== 'suspended') failed = true;
      changed();
      return false;
    }
  }

  async function testOutput(id) {
    const generation = ++testGeneration;
    for (const voice of [...voices]) if (voice.test) stopVoice(voice);
    const sound = sounds.get(id);
    if (!sound) return false;
    testState = 'loading';
    changed();
    if (!await enable()) { testState = 'idle'; changed(); return false; }
    if (!sound.buffer) await sound.pending;
    if (generation !== testGeneration || sounds.get(id) !== sound || disposed) return false;
    testState = 'idle';
    if (!sound.buffer) { changed(); return false; }
    // [ai] A two-second maximum is a local output-test safety bound, not a
    // sound-design envelope. Tests use the existing Music sample and mix.
    if (audioSoundGain(mix, sound.category, sound.level) === 0) { changed(); return false; }
    return play(sound, { duration: 2, test: true });
  }

  function snapshot() {
    const entries = [...sounds.values()];
    const status = unavailable || context?.state === 'closed' ? 'unavailable'
      : failed || entries.some(sound => sound.failed) ? 'failed'
      : context && context.state !== 'running' ? 'blocked'
      : voices.size ? 'playing'
      : entries.some(sound => sound.loading) ? 'loading' : 'idle';
    return {
      status, test: testState, reducedRange,
      reducedRangeAvailable: range?.available === true,
      categories: AUDIO_CATEGORIES.filter(id => entries.some(sound => sound.category === id)),
      active: [...voices].map(voice => ({ id: voice.sound.id, category: voice.sound.category, test: voice.test })),
      ready: entries.filter(sound => sound.buffer).map(sound => sound.id),
      levels: Object.fromEntries(entries.map(sound => [sound.id, sound.level])),
    };
  }

  function outputPeak() {
    if (!meter) return 0;
    const values = new Float32Array(meter.fftSize);
    meter.getFloatTimeDomainData(values);
    return values.reduce((peak, value) => Math.max(peak, Math.abs(value)), 0);
  }
  function dispose() {
    stopAll();
    disposed = true;
    sounds.clear();
    cache.clear();
    range?.dispose();
    if (context) {
      context.onstatechange = null;
      Promise.resolve(context.close()).catch(() => {});
    }
  }
  return { register, remove, loop, cue, setMix, setMono, setDucking, setReducedRange, enable, testOutput, stopAll, snapshot, outputPeak, dispose };
}
