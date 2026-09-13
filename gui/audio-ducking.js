/** Local presentation envelope. One current window, no cue queue/history.
 * Overlaps extend the hold only until its authored cap; the complete release
 * then runs even if more alerts arrive. All times use the provider sample clock. */
export function validDuckingSpec(value) {
  return value && ['gain', 'attack_seconds', 'hold_seconds', 'max_hold_seconds', 'release_seconds']
    .every(key => Number.isFinite(value[key])) && value.gain >= 0 && value.gain <= 1
    && value.attack_seconds > 0 && value.hold_seconds >= value.attack_seconds
    && value.max_hold_seconds >= value.hold_seconds && value.release_seconds > 0;
}
export function duckGain(window, now, spec) {
  if (!window || now >= window.end) return 1;
  if (now < window.attackEnd) return window.from + (window.floor - window.from)
    * Math.max(0, (now - window.attackStart) / (window.attackEnd - window.attackStart));
  if (now <= window.hold) return window.floor;
  return window.floor + (1 - window.floor) * (now - window.hold) / spec.release_seconds;
}
export function nextDuck(window, now, spec) {
  if (!validDuckingSpec(spec)) return null;
  if (window && now < window.end && now >= window.cap) return window;
  const continuing = window && now < window.end;
  const cap = continuing ? window.cap : now + spec.max_hold_seconds;
  const hold = Math.min(cap, Math.max(continuing ? window.hold : now, now + spec.hold_seconds));
  const from = duckGain(window, now, spec);
  return { cap, from, floor: spec.gain, attackStart: now, attackEnd: Math.min(now + spec.attack_seconds, hold),
    hold, end: hold + spec.release_seconds };
}
export function releaseDuck(window, now, spec) {
  const from = duckGain(window, now, spec);
  return from === 1 ? null : { cap: now, from, floor: from, attackStart: now, attackEnd: now,
    hold: now, end: now + spec.release_seconds };
}
