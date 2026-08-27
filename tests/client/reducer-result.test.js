// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import {
  CHANGE_DOMAINS,
  emptyReducerResult,
  mergeReducerResults,
} from '../../gui/reducer-result.js';

describe('mergeReducerResults', () => {
  it('merges semantic sets in first-seen order and coalesces duplicates', () => {
    const first = emptyReducerResult();
    first.changedBlackboards.add('scan');
    first.changedDomains.add(CHANGE_DOMAINS.STATION_HOSTING);
    const second = emptyReducerResult();
    second.changedBlackboards.add('scan');
    second.changedBlackboards.add('helm');
    second.changedSystems.add('helm-thrust');

    const merged = mergeReducerResults(undefined, first, second);

    expect([...merged.changedBlackboards]).toEqual(['scan', 'helm']);
    expect([...merged.changedSystems]).toEqual(['helm-thrust']);
    expect([...merged.changedDomains]).toEqual([CHANGE_DOMAINS.STATION_HOSTING]);
  });

  it('returns fresh empty sets when no reducer reports a change', () => {
    const a = mergeReducerResults();
    const b = mergeReducerResults(null, undefined);
    a.changedBlackboards.add('power');
    expect([...b.changedBlackboards]).toEqual([]);
  });

  it('exposes the same merger to the non-module shell', () => {
    expect(window.mergeReducerResults).toBe(mergeReducerResults);
  });
});
