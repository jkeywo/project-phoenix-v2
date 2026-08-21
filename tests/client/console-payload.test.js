/**
 * tests/client/console-payload.test.js — gui/console-payload.js (issue #1231,
 * T4.C0 of the console-seam programme).
 *
 * Pure-Node tests over `systemView`, the shared replacement for the
 * `function system(s, ...ids) { ... }` helper copy-pasted into 9 console
 * documents (e.g. gui/cruiser/engineering.html). Nothing consumes this
 * module yet — phase 1 of the programme swaps each copy for an import — so
 * these assertions pin the exact semantics the 9 copies share today.
 */
import { describe, it, expect } from 'vitest';
import { systemView } from '../../gui/console-payload.js';

describe('systemView', () => {
  it('returns the first present system among the candidate ids', () => {
    const s = { systems: { 'power-reactor': { level: 3 }, repair: { crews: 2 } } };
    expect(systemView(s, 'power-reactor', 'power-battery')).toEqual({ level: 3 });
    expect(systemView(s, 'shields-system', 'repair')).toEqual({ crews: 2 });
  });

  it('skips ids not present and falls through to a later candidate', () => {
    const s = { systems: { repair: { crews: 1 } } };
    expect(systemView(s, 'power-reactor', 'power-battery', 'repair')).toEqual({ crews: 1 });
  });

  it('returns {} when none of the candidate ids are present', () => {
    const s = { systems: { repair: {} } };
    expect(systemView(s, 'power-reactor', 'shields-system')).toEqual({});
  });

  it('returns {} when s.systems is missing entirely', () => {
    expect(systemView({})).toEqual({});
    expect(systemView({}, 'power-reactor')).toEqual({});
  });

  it('returns {} when called with no candidate ids at all', () => {
    const s = { systems: { repair: { crews: 1 } } };
    expect(systemView(s)).toEqual({});
  });

  it('treats a present-but-falsy value as absent and keeps looking (matches the copy-pasted helper)', () => {
    const s = { systems: { 'power-reactor': null, repair: { crews: 4 } } };
    expect(systemView(s, 'power-reactor', 'repair')).toEqual({ crews: 4 });
  });

  it('is a plain pure function importable in Node with no DOM globals', () => {
    expect(typeof systemView).toBe('function');
  });
});
