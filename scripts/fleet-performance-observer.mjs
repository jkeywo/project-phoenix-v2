// Install with page.evaluate(installBrowserPerformanceObserver, { peer, clock })
// after the matrix's existing observer is present. Capture is opt-in and does
// not change game messages. Read the rows before closing the document.
export function installBrowserPerformanceObserver({ peer, clock }) {
  if (!peer || !clock || !window.__matrixEvidence?.outcomes) {
    throw new Error('Install after the matrix observer with explicit peer and clock identities');
  }
  if (window.__fleetPerformance) throw new Error('Performance observer already installed');
  const events = [];
  const at = () => performance.now();
  const add = (kind, correlation, tick, agreed) => {
    const row = { kind, peer, clock, ms: at() };
    if (correlation !== undefined) row.correlation = correlation;
    if (tick !== undefined) row.tick = tick;
    if (agreed !== undefined) row.agreed = agreed;
    events.push(row);
  };
  // The matrix observer slices this array after every message, replacing the
  // property. Rewrap each replacement so a long run does not lose capture.
  const evidence = window.__matrixEvidence;
  let outcomes;
  const wrap = value => {
    if (!Array.isArray(value)) throw new Error('Matrix outcomes must remain an array');
    const push = value.push;
    value.push = function (...values) {
      for (const item of values) if (item?.outcome === 'Applied' && item.correlation) {
        // The authoritative application occurred earlier, but its time is
        // not inferred from this returned receipt.
        add('applied_receipt', item.correlation);
      }
      return push.apply(this, values);
    };
    return value;
  };
  outcomes = wrap(evidence.outcomes);
  Object.defineProperty(evidence, 'outcomes', {
    configurable: true, enumerable: true,
    get() { return outcomes; }, set(value) { outcomes = wrap(value); },
  });
  window.__fleetPerformance = Object.freeze({
    input(correlation) { add('input', correlation); },
    tick(tick) { add('tick', undefined, tick); },
    mark(kind, correlation, tick, agreed) { add(kind, correlation, tick, agreed); },
    read() { return events.map(event => ({ ...event })); },
  });
}

// Host-side observation of the production mesh egress. Every local tick frame
// is stamped when JS drains it; no tick or applied command is manufactured.
export function installHostPerformanceObserver({ peer, clock }) {
  if (!peer || !clock || typeof window.wasm_take_mesh_frames !== 'function'
      || typeof window.__hostMeshStatus !== 'function') {
    throw new Error('Install after a real fleet host exposes its mesh egress');
  }
  if (window.__fleetHostPerformance) throw new Error('Host performance observer already installed');
  const events = [];
  const add = (kind, correlation, tick, agreed) => {
    const row = { kind, peer, clock, ms: performance.now() };
    if (correlation !== undefined) row.correlation = correlation;
    if (tick !== undefined) row.tick = tick;
    if (agreed !== undefined) row.agreed = agreed;
    events.push(row);
  };
  const localSlot = window.__hostMeshStatus().slot;
  const take = window.wasm_take_mesh_frames;
  window.wasm_take_mesh_frames = function (...args) {
    const raw = take(...args);
    for (const frame of JSON.parse(raw)) {
      if (frame.t === 'tick' && frame.d?.from === localSlot && Number.isSafeInteger(frame.d.tick)) {
        add('tick', undefined, frame.d.tick);
      }
    }
    return raw;
  };
  let watch = null;
  window.__fleetHostPerformance = Object.freeze({
    fault(correlation, victimSlot, afterTick) {
      if (watch) throw new Error('Recovery watch already active');
      add('fault', correlation);
      let detected = false, resumed = false;
      watch = setInterval(() => {
        const state = window.__hostMeshStatus();
        if (!detected) {
          const loss = state.recovery?.losses?.find(row => row.slot === victimSlot);
          if (loss) { add('loss_detected', correlation, loss.tick); detected = true; }
        }
        if (!resumed && state.tick > afterTick + 30 && state.peers?.length === 4) {
          add('progress_resumed', correlation, state.tick); resumed = true;
        }
        if (detected && resumed) { clearInterval(watch); watch = null; }
      }, 25);
    },
    verified(correlation) {
      const state = window.__hostMeshStatus();
      if (!state.agreed || !Number.isSafeInteger(state.tick)) throw new Error('No verified host digest');
      add('digest_verified', correlation, state.tick, true);
    },
    read() { return events.map(event => ({ ...event })); },
    stop() { if (watch) clearInterval(watch); watch = null; },
  });
}
