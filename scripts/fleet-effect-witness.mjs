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
      if (newest < oldLast) fail('backward-history');
      // Keep a complete overlapping tick; the bounded ring may remove a
      // prefix, including part of its oldest tick, but never a middle row.
      if (oldest > oldFirst && oldest >= oldLast) fail('sampling-gap-no-complete-overlap');
      // ActivityHistory sorts late ordinary combat rows by their actual tick.
      // A prelaunch connection can therefore have a later tick than combat
      // facts subsequently inserted into the same bounded history. Match all
      // previously witnessed rows as an exact suffix; new unrelated rows may
      // appear anywhere, but an old retained row cannot silently change.
      const remaining = new Map();
      for (const row of rows) remaining.set(key(row), (remaining.get(key(row)) || 0) + 1);
      const retained = previous.map(() => false);
      for (let i = previous.length - 1; i >= 0; i--) {
        const encoded = key(previous[i]), count = remaining.get(encoded) || 0;
        if (count) { retained[i] = true; remaining.set(encoded, count - 1); }
      }
      const firstRetained = retained.indexOf(true);
      if (firstRetained < 0 || retained.slice(firstRetained).some(value => !value))
        fail('changed-or-partially-evicted-tick');
      // If the old history contained the new oldest tick, at least one row
      // from that tick must still anchor it. A wholly replaced oldest tick
      // cannot prove whether its prior rows were evicted or rewritten.
      if (previous.some(row => row.tick === oldest)
        && !previous.some((row, i) => row.tick === oldest && retained[i]))
        fail('changed-or-partially-evicted-tick');
      removed = firstRetained < 0 ? previous.length : firstRetained;
      // A newly inserted older row cannot evict a later retained history row:
      // the ring always discards its lowest tick first.
      if (removed && previous[removed - 1].tick > oldest)
        fail('changed-or-partially-evicted-tick');
      if (removed && rows.length !== capacity) fail('non-capacity-eviction');
      const oldCounts = new Map();
      for (const row of previous) oldCounts.set(key(row), (oldCounts.get(key(row)) || 0) + 1);
      added = [];
      for (const row of rows) {
        const encoded = key(row), count = oldCounts.get(encoded) || 0;
        if (count) oldCounts.set(encoded, count - 1);
        else {
          if (row.tick < oldLast && isEffect(row)) fail('backdated-target-effect');
          added.push(row);
        }
      }
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
