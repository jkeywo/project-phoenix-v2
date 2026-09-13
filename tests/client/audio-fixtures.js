/** Signal-carrying device fake: the production provider builds its real graph
 * against this API. Tests sample the sink after gains, rather than asserting a
 * setter or checking HTMLMediaElement.volume. Chromium covers actual decoding. */
export class FakeAudioContext {
  constructor({ state = 'suspended', refuseResume = false, rejectDecode = false, rejectStart = false } = {}) {
    Object.assign(this, { state, refuseResume, rejectDecode, rejectStart, currentTime: 0 });
    this.sources = [];
    this.destination = { sink: true };
  }
  node(extra = {}) {
    return Object.assign({
      targets: [],
      connect(target) { this.targets.push(target); return target; },
      disconnect() { this.targets = []; },
    }, extra);
  }
  createGain() { return this.node({ gain: { value: 1 } }); }
  createPanner() {
    return this.node({ positionX: { value: 0 }, positionY: { value: 0 }, positionZ: { value: 0 } });
  }
  createAnalyser() {
    return this.node({ fftSize: 256, getFloatTimeDomainData: values => values.fill(this.sample()) });
  }
  createBufferSource() {
    const context = this;
    const source = this.node({
      loop: false, started: false, stopAt: Infinity,
      start() {
        if (context.rejectStart) throw new Error('Device could not start source');
        this.started = true; this.startAt = context.currentTime;
      },
      stop(at) {
        this.stopAt = at ?? context.currentTime;
        if (this.stopAt <= context.currentTime) { this.started = false; this.onended?.(); }
      },
    });
    this.sources.push(source);
    return source;
  }
  async decodeAudioData(bytes) {
    if (this.rejectDecode) throw new Error('Unsupported codec');
    return { duration: 10, sample: new Float32Array(bytes)[0] };
  }
  async resume() {
    if (this.refuseResume) throw new Error('Gesture required');
    this.state = 'running'; this.onstatechange?.();
  }
  async close() { this.state = 'closed'; }
  sample() {
    if (this.state !== 'running') return 0;
    function through(node, value) {
      if (node.sink) return value;
      return node.targets.reduce((sum, target) => sum + through(target, value * (node.gain?.value ?? 1)), 0);
    }
    return this.sources.filter(source => source.started).reduce((sum, source) => sum + through(source, source.buffer.sample), 0);
  }
  advance(seconds) {
    this.currentTime += seconds;
    for (const source of this.sources) {
      if (source.started && (this.currentTime >= source.stopAt
        || (!source.loop && this.currentTime >= source.startAt + source.buffer.duration))) {
        source.started = false; source.onended?.();
      }
    }
  }
}

export function audioFetch(sample = 0.5) {
  return async () => ({ ok: true, arrayBuffer: async () => new Float32Array([sample]).buffer });
}
export function memoryStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: key => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: key => values.delete(key),
    values,
  };
}
export async function settleAudio() { for (let i = 0; i < 16; i++) await Promise.resolve(); }
export const AUDIO_CONFIG = {
  ambient: { file: 'Ambient.mp3', volume: 0.25 },
  engine: { file: 'Engine.mp3', idle_volume: 0.1, volume_at_full_thrust: 0.15 },
  phaser_loop: { file: 'PhaserLoop.mp3', volume: 0.5 },
  forcefield: { file: 'ForcefieldHit.mp3' },
  blaster: { file: 'Blaster.mp3', volume: 0.9, ref_distance: 30, max_distance: 800,
    rolloff_factor: 1.2, distance_model: 'inverse', panning_model: 'equalpower' },
  red_alert: { siren_file: 'siren.ogg', siren_volume: 0.6, music_file: 'music.ogg', music_volume: 0.4 },
  computer_message: { info: { file: 'info.ogg', volume: 0.2 }, advisory: { file: 'advisory.ogg', volume: 0.25 }, warning: { file: 'warning.ogg', volume: 0.3 }, critical: { file: 'critical.ogg', volume: 0.7 } },
};
