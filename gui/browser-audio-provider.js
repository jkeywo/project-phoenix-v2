import { AUDIO_CATEGORIES, normalizeAudioMix, audioBusGain, audioSoundGain, clampAudioLevel } from './audio-mix.js';

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
  const buses = new Map();
  const sounds = new Map();
  const voices = new Set();
  const cache = new Map();
  function changed() { onChange(); }

  function graph() {
    if (unavailable || disposed) return null;
    if (context) return context;
    try {
      context = contextFactory();
      if (!context) { unavailable = true; changed(); return null; }
      master = context.createGain();
      meter = context.createAnalyser();
      meter.fftSize = 256;
      master.connect(meter).connect(context.destination);
      for (const id of AUDIO_CATEGORIES) {
        const gain = context.createGain();
        gain.connect(master);
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
    mix = normalizeAudioMix(value);
    if (master) master.gain.value = audioBusGain(mix, 'master');
    for (const [id, gain] of buses) gain.gain.value = audioBusGain(mix, id);
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
    testGeneration++;
    testState = 'idle';
    for (const sound of sounds.values()) sound.wanted = false;
    for (const voice of [...voices]) stopVoice(voice);
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
      status, test: testState,
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
    if (context) {
      context.onstatechange = null;
      Promise.resolve(context.close()).catch(() => {});
    }
  }
  return { register, remove, loop, cue, setMix, enable, testOutput, stopAll, snapshot, outputPeak, dispose };
}
