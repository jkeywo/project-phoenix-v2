import { describe, expect, it } from 'vitest';
import { FORMAT, summarize } from '../../scripts/fleet-performance-evidence.mjs';
import { installBrowserPerformanceObserver } from '../../scripts/fleet-performance-observer.mjs';

const provenance = { revision: 'a'.repeat(40), content: 'probe@1', runtime: { browser: 'test' },
  artifactHashes: { bundle: 'b'.repeat(64) }, profile: { route: 'direct' } };
const event = (kind, ms, extra = {}) => ({ kind, ms, clock: 'client-clock', peer: 'ship-1/helm', ...extra });
const trace = events => ({ format: FORMAT, provenance, expectedTickMs: 16, events });

describe('T5 performance evidence', () => {
  it('separates genuine application from return-inclusive receipt and never subtracts another clock', () => {
    const result = summarize(trace([
      event('input', 10, { correlation: 'a' }),
      event('authoritative_applied', 13, { correlation: 'a', tick: 12, clock: 'host-clock', peer: 'ship-1' }),
      event('applied_receipt', 18, { correlation: 'a' }),
    ]));
    expect(result.input_to_authoritative_application.distribution).toBeNull();
    expect(result.input_to_applied_receipt.distribution).toMatchObject({ count: 1, p95_ms: 8 });
    expect(result.input_to_authoritative_application.incomplete).toHaveLength(2);
  });

  it('reports application, unplanned gaps, deliberate pause and verified recovery from one observed clock', () => {
    const result = summarize(trace([
      event('input', 1, { correlation: 'a' }),
      event('authoritative_applied', 9, { correlation: 'a', tick: 4 }),
      event('tick', 0, { tick: 1 }), event('tick', 16, { tick: 2 }),
      event('tick', 50, { tick: 3 }),
      event('pause_start', 55), event('tick', 60, { tick: 4 }),
      event('tick', 100, { tick: 5 }), event('pause_end', 105),
      event('fault', 110, { correlation: 'loss-1' }),
      event('loss_detected', 125, { correlation: 'loss-1' }),
      event('restore_commit', 130, { correlation: 'loss-1', tick: 6 }),
      event('progress_resumed', 150, { correlation: 'loss-1', tick: 7 }),
      event('digest_verified', 160, { correlation: 'loss-1', tick: 8, agreed: true }),
    ]));
    expect(result.input_to_authoritative_application.distribution.p95_ms).toBe(8);
    expect(result.stalls.samples.filter(row => row.unplanned_excess_ms > 0)).toHaveLength(1);
    expect(result.stalls.per_observer[0].unplanned_excess_ms).toBe(18);
    expect(result.stalls.longest_unplanned_ms).toBe(18);
    expect(result.recovery.fault_to_loss_detection.distribution.p95_ms).toBe(15);
    expect(result.recovery.fault_to_progress.distribution.p95_ms).toBe(40);
    expect(result.recovery.fault_to_verified_digest.distribution.p95_ms).toBe(50);
  });

  it('requires provenance, correlation and observed application tick', () => {
    expect(() => summarize(trace([event('authoritative_applied', 1, { correlation: 'a' })])))
      .toThrow('observed tick');
    expect(() => summarize({ ...trace([]), provenance: {} })).toThrow('required');
    expect(() => summarize({ ...trace([]), provenance: { ...provenance, artifactHashes: { bundle: 'bad' } } }))
      .toThrow('SHA-256');
    expect(() => summarize(trace([event('digest_verified', 1, { correlation: 'f', tick: 2, agreed: false })])))
      .toThrow('agreement');
    expect(() => summarize(trace([event('tick', 0, { tick: 1 }), event('tick', 40, { tick: 3 })])))
      .toThrow('Noncontiguous tick');
    expect(() => summarize(trace([event('pause_start', 0), event('tick', 1, { tick: 1 })])))
      .toThrow('Unclosed pause');
  });

  it('keeps one stalled peer visible instead of diluting it across fleet duration', () => {
    const slowTicks = [0, 100, 200, 300, 400, 600, 700, 800, 900, 1000];
    const events = [
      ...slowTicks.map((ms, index) => event('tick', ms, { tick: index + 1, peer: 'slow' })),
      ...Array.from({ length: 10 }, (_, index) =>
        event('tick', index * 100, { tick: index + 1, peer: 'fast' })),
    ];
    const result = summarize({ ...trace(events), expectedTickMs: 100 });
    const slow = result.stalls.per_observer.find(row => row.peer === 'slow');
    const fast = result.stalls.per_observer.find(row => row.peer === 'fast');
    expect(slow.unplanned_fraction).toBe(.1);
    expect(fast.unplanned_fraction).toBe(0);
    expect(result.stalls.worst_observer_fraction).toBe(slow.unplanned_fraction);
  });

  it('records an actual matrix outcome on the input page clock without inventing host time', () => {
    const previousWindow = globalThis.window;
    const previousPerformance = globalThis.performance;
    let now = 5;
    globalThis.window = { __matrixEvidence: { outcomes: [] } };
    globalThis.performance = { now: () => now };
    try {
      installBrowserPerformanceObserver({ peer: 'ship-1/helm', clock: 'doc-1' });
      window.__fleetPerformance.input('action-1');
      now = 20;
      window.__matrixEvidence.outcomes.push({ correlation: 'action-1', outcome: 'Applied' });
      window.__matrixEvidence.outcomes = window.__matrixEvidence.outcomes.slice(-64);
      now = 30;
      window.__matrixEvidence.outcomes.push({ correlation: 'action-2', outcome: 'Applied' });
      expect(window.__fleetPerformance.read()).toEqual([
        { kind: 'input', peer: 'ship-1/helm', clock: 'doc-1', ms: 5, correlation: 'action-1' },
        { kind: 'applied_receipt', peer: 'ship-1/helm', clock: 'doc-1', ms: 20, correlation: 'action-1' },
        { kind: 'applied_receipt', peer: 'ship-1/helm', clock: 'doc-1', ms: 30, correlation: 'action-2' },
      ]);
    } finally {
      globalThis.window = previousWindow;
      globalThis.performance = previousPerformance;
    }
  });
});
