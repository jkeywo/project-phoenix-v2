// Pure acceptance evidence: collection adapters retain observed slots, frames and times.
const integer = value => Number.isSafeInteger(value) && value >= 0;
const digest = value => typeof value === 'string' && /^[0-9a-f]{16}$/.test(value);

export function evaluateFleetDigests(peers, { afterTick = -1, afterTime } = {}) {
  const malformed = [], conflicts = [], labels = new Set(), slots = new Set();
  const observed = new Map(), eligible = new Map();
  const cutoff = afterTime === undefined ? null : Date.parse(afterTime);
  if (cutoff !== null && !Number.isFinite(cutoff)) malformed.push({ reason: 'invalid-cutoff' });
  for (const peer of peers) {
    if (typeof peer.label !== 'string' || !peer.label || labels.has(peer.label)
        || !integer(peer.slot) || slots.has(peer.slot)) {
      malformed.push({ peer: peer.label, reason: 'invalid-peer' });
    }
    labels.add(peer.label); slots.add(peer.slot);
    for (const [index, frame] of (peer.frames || []).entries()) {
      if (frame?.t !== 'digest') continue;
      const d = frame.d;
      const at = cutoff === null ? null : Date.parse(frame.at);
      if (!d || d.from !== peer.slot || !integer(d.tick) || !digest(d.digest)
          || (cutoff !== null && (typeof frame.at !== 'string' || !Number.isFinite(at)))) {
        malformed.push({ peer: peer.label, index, reason: 'invalid-digest' });
        continue;
      }
      if (!observed.has(d.tick)) observed.set(d.tick, new Map());
      const seen = observed.get(d.tick);
      if (seen.has(peer.label) && seen.get(peer.label) !== d.digest) {
        conflicts.push({ peer: peer.label, tick: d.tick, first: seen.get(peer.label), next: d.digest });
      } else seen.set(peer.label, d.digest);
      if (d.tick <= afterTick || (cutoff !== null && at <= cutoff)) continue;
      if (!eligible.has(d.tick)) eligible.set(d.tick, new Map());
      eligible.get(d.tick).set(peer.label, d.digest);
    }
  }
  const common = [...eligible].sort(([a], [b]) => a - b)
    .filter(([, rows]) => rows.size === peers.length && peers.every(peer => rows.has(peer.label)))
    .map(([tick, rows]) => ({ tick, byPeer: Object.fromEntries(peers.map(peer => [peer.label, rows.get(peer.label)])),
      agreed: new Set(rows.values()).size === 1 }));
  return { common, malformed, conflicts,
    passed: peers.length > 0 && !malformed.length && !conflicts.length && common.length >= 2 && common.every(row => row.agreed) };
}
