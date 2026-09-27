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
