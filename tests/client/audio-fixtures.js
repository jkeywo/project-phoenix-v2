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
  createGain() {
    const context = this; let value = 1; let events = [];
    const gain = {
      get value() {
        let before = { value, time: 0 };
        for (const event of events) {
          if (event.time > context.currentTime) {
            if (event.ramp) return before.value + (event.value - before.value)
              * Math.max(0, (context.currentTime - before.time) / (event.time - before.time));
            return before.value;
          }
          before = event;
        }
        return before.value;
      },
      set value(next) { value = next; events = []; },
      cancelScheduledValues(time) { events = events.filter(event => event.time < time); },
      setValueAtTime(value, time) { events.push({ value, time }); return this; },
      linearRampToValueAtTime(value, time) { events.push({ value, time, ramp: true }); return this; },
    };
    return this.node({ gain });
  }
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
    const samples = new Float32Array(bytes);
    return { duration: 10, sample: samples[0], stereo: [samples[0], samples[1] ?? samples[0]] };
  }
  async resume() {
    if (this.refuseResume) throw new Error('Gesture required');
    this.state = 'running'; this.onstatechange?.();
  }
  async close() { this.state = 'closed'; }
  sampleChannels() {
    if (this.state !== 'running') return [0, 0];
    function through(node, value) {
      if (node.sink) return value;
      if (node.channelCountMode === 'explicit' && node.channelCount === 1) value = [(value[0] + value[1]) / 2, (value[0] + value[1]) / 2];
      const scaled = value.map(sample => sample * (node.gain?.value ?? 1));
      return node.targets.reduce((sum, target) => {
        const output = through(target, scaled);
        return sum.map((sample, channel) => sample + output[channel]);
      }, [0, 0]);
    }
    return this.sources.filter(source => source.started).reduce((sum, source) => {
      const output = through(source, source.buffer.stereo);
      return sum.map((sample, channel) => sample + output[channel]);
    }, [0, 0]);
  }
  sample() { return this.sampleChannels().reduce((a, b) => a + b, 0) / 2; }
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
