import spec from './audio-range-data.js';

/** Optional stage before Master. The disabled route is an ordinary wire.
 * Recreating the compressor when enabled discards its short internal delay;
 * disabling never leaves a disconnected processor retaining an old cue. */
export function createAudioRange(context, input, output) {
  let nodes = [];
  let enabled = false;
  let available = typeof context.createDynamicsCompressor === 'function'
    && typeof context.createWaveShaper === 'function';
  input.connect(output);

  function disconnect() {
    try { input.disconnect(); } catch (_) { /* already detached */ }
    for (const node of nodes) { try { node.disconnect(); } catch (_) { /* already detached */ } }
    nodes = [];
  }
  function set(value) {
    const wanted = value === true && available;
    if (wanted === enabled) return enabled;
    disconnect();
    enabled = false;
    if (wanted) {
      try {
        const compressor = context.createDynamicsCompressor();
        compressor.threshold.value = spec.threshold_db;
        compressor.knee.value = spec.knee_db;
        compressor.ratio.value = spec.ratio;
        compressor.attack.value = spec.attack_seconds;
        compressor.release.value = spec.release_seconds;
        const compensation = context.createGain();
        compensation.gain.value = spec.browser_compensation;
        const ceiling = context.createWaveShaper();
        // A final bounded transfer also protects abrupt or summed peaks while
        // the compressor attacks. It is inactive below the authored ceiling.
        ceiling.curve = Float32Array.from({ length: 2049 }, (_, i) =>
          Math.max(-spec.ceiling, Math.min(spec.ceiling, i / 1024 - 1)));
        nodes = [compressor, compensation, ceiling];
        input.connect(compressor).connect(compensation).connect(ceiling).connect(output);
        enabled = true;
      } catch (_) {
        disconnect();
        available = false;
      }
    }
    if (!enabled) input.connect(output);
    return enabled;
  }
  return {
    set,
    get available() { return available; },
    get enabled() { return enabled; },
    reset() { if (enabled) { set(false); set(true); } },
    dispose() { disconnect(); enabled = false; },
  };
}
