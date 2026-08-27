// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import {
  CHANGE_DOMAINS,
  REDUCER_EFFECTS,
  emptyReducerResult,
  mergeReducerResults,
} from '../../gui/reducer-result.js';

describe('mergeReducerResults', () => {
  it('merges semantic sets in first-seen order and coalesces duplicates', () => {
    const first = emptyReducerResult();
    first.changedBlackboards.add('scan');
    first.changedDomains.add(CHANGE_DOMAINS.STATION_HOSTING);
    first.effects.push({ effect: REDUCER_EFFECTS.VIBRATE, duration: 80 });
    const second = emptyReducerResult();
    second.changedBlackboards.add('scan');
    second.changedBlackboards.add('helm');
    second.changedSystems.add('helm-thrust');
    second.effects.push(
      { effect: REDUCER_EFFECTS.VIBRATE, duration: 80 },
      { effect: REDUCER_EFFECTS.REQUEST_RENDER },
    );

    const merged = mergeReducerResults(undefined, first, second);

    expect([...merged.changedBlackboards]).toEqual(['scan', 'helm']);
    expect([...merged.changedSystems]).toEqual(['helm-thrust']);
    expect([...merged.changedDomains]).toEqual([CHANGE_DOMAINS.STATION_HOSTING]);
    expect(merged.effects).toEqual([
      { effect: REDUCER_EFFECTS.VIBRATE, duration: 80 },
      { effect: REDUCER_EFFECTS.VIBRATE, duration: 80 },
      { effect: REDUCER_EFFECTS.REQUEST_RENDER },
    ]);
  });

  it('returns fresh empty sets when no reducer reports a change', () => {
    const a = mergeReducerResults();
    const b = mergeReducerResults(null, undefined);
    a.changedBlackboards.add('power');
    a.effects.push({ effect: 'test-only' });
    expect([...b.changedBlackboards]).toEqual([]);
    expect(b.effects).toEqual([]);
  });

  it('exposes the same merger to the non-module shell', () => {
    expect(window.mergeReducerResults).toBe(mergeReducerResults);
  });
});
