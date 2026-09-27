/** Test-only impairment at real RTCDataChannel.send. Never installed by Phoenix.
 * It preserves reliable FIFO and samples loss only on the snapshot channel.
 * Counters describe handoff to RTC, not packet loss or remote acknowledgement.
 */
export function impairDataChannel(channel, profile, counters, clock = {}) {
  if (!['reliable', 'snapshot'].includes(channel.label)) return;
  const now = clock.now || (() => performance.now());
  const schedule = clock.schedule || setTimeout;
  const cancel = clock.cancel || clearTimeout;
  const kind = channel.label;
  const stats = counters[kind] ||= { seen: 0, written: 0, dropped: 0, cancelled: 0,
    delay: { count: 0, min: null, max: null, total: 0 } };
  counters.pending ||= 0; counters.peakPending ||= 0; counters.overflows ||= 0; counters.failures ||= 0;
  let random = profile.seed >>> 0;
  let timer = null;
  const queue = [];
  const send = channel.send.bind(channel);
  function write(payload, at) {
    try {
      send(payload); stats.written++;
      const elapsed = now() - at;
      stats.delay.count++; stats.delay.total += elapsed;
      stats.delay.min = stats.delay.min === null ? elapsed : Math.min(stats.delay.min, elapsed);
      stats.delay.max = stats.delay.max === null ? elapsed : Math.max(stats.delay.max, elapsed);
    } catch {
      stats.cancelled++; counters.failures++; channel.close();
    }
  }
  function pump() {
    timer = null;
    while (queue.length && queue[0].due <= now()) {
      const entry = queue.shift(); counters.pending--;
      if (channel.readyState === 'open') write(entry.payload, entry.at);
      else stats.cancelled++;
    }
    if (queue.length) timer = schedule(pump, Math.max(0, queue[0].due - now()));
  }
  channel.addEventListener('close', () => {
    if (timer !== null) cancel(timer);
    timer = null; stats.cancelled += queue.length; counters.pending -= queue.length; queue.length = 0;
  });
  channel.send = payload => {
    // Keep the browser's ordinary invalid-state behaviour before queueing.
    if (channel.readyState !== 'open') return send(payload);
    stats.seen++;
    if (kind === 'snapshot') {
      random = (Math.imul(random, 1664525) + 1013904223) >>> 0;
      if (random / 0x100000000 < profile.lossPercent / 100) { stats.dropped++; return; }
    }
    const at = now();
    if (!profile.delayMs) { write(payload, at); return; }
    if (counters.pending >= 512) {
      counters.overflows++;
      if (kind === 'snapshot') stats.dropped++;
      else { stats.cancelled++; channel.close(); }
      return;
    }
    queue.push({ payload, at, due: at + profile.delayMs });
    counters.pending++; counters.peakPending = Math.max(counters.peakPending, counters.pending);
    if (timer === null) timer = schedule(pump, profile.delayMs);
  };
}
