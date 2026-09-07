import { expect, it } from 'vitest';
import { summarizeSystems } from '../../scripts/profile-systems-analysis.mjs';

it('keeps deferred/nested work separate and correlates spikes by exact update index', () => {
  const result = summarizeSystems({
    updates: [{ update: 300, tick: 299, durationMs: 30 }, { update: 301, tick: 300, durationMs: 1 }],
    systems: [{ name: 'physics', schedule: 'FixedUpdate', category: 'Physics', counts: {
      calls: 2, nanos: 25e6, max_ns: 24e6, deferred_calls: 1, deferred_ns: 2e6,
      spans: [{ update: 300, start_ns: 0, end_ns: 24e6, deferred: false },
        { update: 300, start_ns: 24e6, end_ns: 26e6, deferred: true },
        { update: 301, start_ns: 31e6, end_ns: 32e6, deferred: false }],
    } }],
    continuation: { tick: 300, digest: '0123456789abcdef' },
  });
  expect(result.systems[0]).toMatchObject({ accumulatedMs: 25, deferredMs: 2 });
  expect(result.slowUpdates[0]).toMatchObject({ update: 300, tick: 299, durationMs: 30 });
  expect(result.slowUpdates[0].spans.map(span => [span.durationMs, span.deferred])).toEqual([[24, false], [2, true]]);
  expect(result.gpuObserved).toBe(false);
});

it('reports only observed GPU paths, with counts and CPU durations distinguished', () => {
  const result = summarizeSystems({ systems: [], updates: [], renderDiagnostics: { samples: [
    { path: 'render/main/elapsed_cpu', value: 2 },
    { path: 'render/main/elapsed_gpu', value: 5 },
    { path: 'render/main/pass/elapsed_gpu', value: 4 },
    { path: 'render/main/pass/fragment_shader_invocations', value: 1000 },
  ] } });
  expect(result.gpuObserved).toBe(true);
  expect(result.renderDiagnostics['render/main/elapsed_gpu']).toMatchObject({ unit: 'ms', mean: 5 });
  expect(result.renderDiagnostics['render/main/pass/fragment_shader_invocations']).toMatchObject({ unit: 'count', mean: 1000 });
  expect(Object.keys(result.renderDiagnostics)).toHaveLength(4);
});
