/**
 * tests/client/console-payload.test.js — gui/console-payload.js (issue #1231,
 * T4.C0, and issue #1233, T4.C1.5, of the console-seam programme).
 *
 * Pure-Node tests over `systemView`, the shared replacement for the
 * `function system(s, ...ids) { ... }` helper copy-pasted into 9 console
 * documents (e.g. gui/cruiser/engineering.html). Nothing consumes this
 * module yet — phase 1 of the programme swaps each copy for an import — so
 * these assertions pin the exact semantics the 9 copies share today.
 *
 * Also covers `normalizeConsolePayload`, the console-core.js normalisation
 * seam that wraps a FLAT payload's fields under `systems[family]` so a
 * console reading through `systemView` never has to know whether the wire
 * payload arrived flat or already keyed (issue #925's defect class).
 */
import { describe, it, expect } from 'vitest';
import { systemView, normalizeConsolePayload } from '../../gui/console-payload.js';

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

// ── normalizeConsolePayload (issue #1233, T4.C1.5) ──────────────────────────
//
// The #925 defect class: `buildConsoleStateInner` emits a FLAT payload for a
// single-family station and a system-id-KEYED payload for a multi-family
// one; a console written against `systemView` finds nothing (falls through
// every candidate to `{}`) when handed the flat shape directly. These tests
// pin the wrap that closes that gap: a flat payload's fields get mirrored
// under `systems[family]`, so `systemView(s, family, ...)` resolves them
// exactly as it would for a genuinely keyed payload.
describe('normalizeConsolePayload', () => {
  it('wraps a flat payload under systems[family], preserving the original top-level fields', () => {
    const flat = { red_alert: true, view_direction: 'Fore', own_hull: { pct: 1 } };
    const out = normalizeConsolePayload(flat, 'captain');
    expect(out.systems.captain).toEqual(flat);
    // Backward-compatible: existing consoles reading top-level fields
    // directly keep working unchanged.
    expect(out.red_alert).toBe(true);
    expect(out.view_direction).toBe('Fore');
    expect(out.own_hull).toEqual({ pct: 1 });
  });

  it('leaves an already-keyed payload (has .systems) unchanged', () => {
    const keyed = { systems: { 'power-reactor': { level: 3 } }, station_id: 'engineering' };
    expect(normalizeConsolePayload(keyed, 'power')).toBe(keyed);
  });

  it('leaves a flat payload unchanged when no family is given', () => {
    const flat = { locked: false };
    const out = normalizeConsolePayload(flat, undefined);
    expect(out).toBe(flat);
    expect(out.systems).toBeUndefined();
  });

  it('passes through null/non-object payloads untouched', () => {
    expect(normalizeConsolePayload(null, 'power')).toBeNull();
    expect(normalizeConsolePayload(undefined, 'power')).toBeUndefined();
  });

  it('#925 regression: systemView resolves the same data whether the wire payload was flat or keyed', () => {
    // Same logical payload, two wire shapes. Before console-core normalised
    // inbound payloads, a console reading via systemView rendered blank for
    // the flat shape — this is the exact defect #925's first pass shipped.
    const flatWire = { battery_charge: 42, battery_max: 100, groups: [] };
    const keyedWire = { systems: { 'power-reactor': { battery_charge: 42, battery_max: 100, groups: [] } } };

    const normalizedFlat = normalizeConsolePayload(flatWire, 'power');
    const normalizedKeyed = normalizeConsolePayload(keyedWire, 'power');

    expect(systemView(normalizedFlat, 'power', 'power-reactor')).toEqual(flatWire);
    expect(systemView(normalizedKeyed, 'power', 'power-reactor')).toEqual(keyedWire.systems['power-reactor']);

    // Without normalisation, the flat wire shape is exactly the historic
    // blank-console symptom: no `.systems`, so every candidate id misses.
    expect(systemView(flatWire, 'power', 'power-reactor')).toEqual({});
  });
});
