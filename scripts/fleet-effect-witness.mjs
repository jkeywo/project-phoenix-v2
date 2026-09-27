// #1534 private runner observer. No production hooks or altered activity payloads.
// Self-contained so Playwright can evaluate the same tested reducer in a page.
export function createEffectWitness({entity, correlation, observe = false,
  intervalMs = 50, maxGapMs = 1000, maxSamples = 3000, maxBytes = 8 * 1024 * 1024,
  maxDurationMs = 120000} = {}) {
  let previous = null, previousAt = null, startedAt = null, capacity = null;
  let samples = 0, bytes = 0, error = null, throughTick = null, rejected = null;
  const events = [], trace = [];
  const fail = reason => { error ||= reason; };
  const key = row => JSON.stringify(row);
  const group = (rows, tick) => rows.filter(row => row.tick === tick).map(key).sort();
  const orderedGroup = (rows, tick) => rows.filter(row => row.tick === tick).map(key);
  const equal = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  const isEffect = row => row.category === 'damage' && row.detail?.type === 'damage'
    && row.detail.data?.weapon === 'gm.direct'
    && row.links?.some(link => link.role === 'victim' && link.entity?.entity_id === entity);
  const read = () => ({error, samples, bytes, throughTick, events: structuredClone(events)});
  function sample(activity, at) {
    if (error) return read();
    if (!Number.isFinite(at) || (previousAt !== null && (at < previousAt || at - previousAt > maxGapMs)))
      fail('sampling-gap-or-clock-rewind');
    startedAt ??= at;
    if (++samples > maxSamples || at - startedAt > maxDurationMs) fail('observer-bound');
    const rows = activity?.entries;
    if (!Array.isArray(rows) || !Number.isSafeInteger(activity.capacity) || activity.capacity < 1
      || activity.capacity > 4096 || rows.length > activity.capacity || !rows.length
      || rows.some((row, i) => !Number.isSafeInteger(row.tick) || row.tick < 0 || (i && row.tick < rows[i - 1].tick)))
      fail('invalid-activity');
    if (error) {
      // Keep the first rejected projection so a failed proof identifies the
      // precise missing or rewritten rows, rather than just its reason code.
      rejected = {at, previous: structuredClone(previous), current: structuredClone(rows)};
      return read();
    }
    if (capacity !== null && capacity !== activity.capacity) fail('capacity-changed');
    capacity = activity.capacity;
    const oldest = rows[0].tick, newest = rows.at(-1).tick;
    let added = rows, removed = 0;
    if (previous) {
      const oldFirst = previous[0].tick, oldLast = previous.at(-1).tick;
      if (oldest < oldFirst || newest < oldLast) fail('backward-history');
      // A strictly earlier retained tick proves the old latest tick is whole.
      // A partial oldest tick is handled below only with exact suffix evidence.
      if (oldest > oldFirst && oldest >= oldLast) fail('sampling-gap-no-complete-overlap');
      for (const tick of new Set(previous.map(row => row.tick))) {
        if (tick < oldest) continue;
        const before = group(previous, tick), after = group(rows, tick);
        if (tick < oldLast && tick === oldest) {
          // The bounded ring evicts individual rows, not whole ticks. Its
          // oldest retained tick may therefore be a suffix of a tick already
          // witnessed. Keep the complete intervening ticks as the continuity
          // anchor and retain evicted effects in `events`.
          const oldOrder = orderedGroup(previous, tick), newOrder = orderedGroup(rows, tick);
          if (!equal(oldOrder.slice(-newOrder.length), newOrder))
            fail('changed-or-partially-evicted-tick');
        } else if (tick < oldLast && !equal(before, after)) {
          fail('changed-or-partially-evicted-tick');
        }
        if (tick === oldLast) {
          const remaining = [...after];
          for (const item of before) {
            const index = remaining.indexOf(item);
            if (index < 0) { fail('changed-or-partially-evicted-tick'); break; }
            remaining.splice(index, 1);
          }
        }
      }
      // New rows may extend only the previous latest tick or later ticks.
      const oldCounts = new Map();
      for (const row of previous) oldCounts.set(key(row), (oldCounts.get(key(row)) || 0) + 1);
      added = [];
      for (const row of rows) {
        const encoded = key(row), count = oldCounts.get(encoded) || 0;
        if (count) oldCounts.set(encoded, count - 1);
        else { if (row.tick < oldLast) fail('backdated-history'); added.push(row); }
      }
      removed = [...oldCounts.values()].reduce((sum, count) => sum + count, 0);
      if (removed && rows.length !== capacity) fail('non-capacity-eviction');
    }
    if (error) {
      rejected = {at, previous: structuredClone(previous), current: structuredClone(rows)};
      return read();
    }
    const record = {at, oldest, newest, removed, added};
    const encoded = JSON.stringify(record);
    bytes += new TextEncoder().encode(encoded).length;
    if (bytes > maxBytes) { fail('observer-byte-bound'); return read(); }
    trace.push(structuredClone(record));
    for (const row of added) if (isEffect(row)) events.push(structuredClone(row));
    if (events.length > 1) fail('duplicate-effect');
    previous = structuredClone(rows); previousAt = at; throughTick = newest;
    return read();
  }
  const observer = {sample, read, finish: () => ({...read(), capacity,
    trace: structuredClone(trace), rejected: structuredClone(rejected)})};
  if (!observe) return observer;
  if (window.__recoveryEffectWitness) throw new Error('Effect observer already installed');
  const poll = () => {
    try { sample(window.__hostGmActivityState?.(), performance.now()); }
    catch (cause) { fail('activity-read-failed: ' + String(cause)); }
  };
  poll();
  const timer = setInterval(poll, intervalMs);
  window.__recoveryEffectWitness = {
    read: () => { poll(); return read(); },
    stop: () => { poll(); clearInterval(timer); return observer.finish(); },
    entity, correlation,
  };
}
