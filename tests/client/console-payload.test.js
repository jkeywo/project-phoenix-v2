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

// ── normalizeConsolePayload (issue #1233, T4.C1.5; correctness fix per review) ─
//
// The #925 defect class: `buildConsoleStateInner` emits a FLAT payload for a
// single-family station and a system-id-KEYED payload for a multi-family
// one; a console written against `systemView` finds nothing (falls through
// every candidate to `{}`) when handed the flat shape directly. These tests
// pin the wrap that closes that gap: a flat payload's fields get mirrored
// under the FINE SYSTEM IDS its family owns (`FAMILY_SYSTEM_IDS`) — the exact
// ids shipped readers pass to `systemView`, and the exact ids
// `buildSystemStationConsoleState` keys a keyed payload by — so
// `systemView(s, '<fine-id>', ...)` resolves a flat payload identically to a
// keyed one.
//
// It must NOT be enough to mirror under the console-family NAME: no shipped
// console ever queries `systemView(s, '<family-name>')` — a wrap keyed by name
// is invisible to `systemView(s, 'power-reactor', ...)`, which is precisely the
// blank-console bug this fix removes.
describe('normalizeConsolePayload', () => {
  it('wraps a flat payload under its family\'s fine system ids (not the family name), preserving top-level fields', () => {
    const flat = { battery_charge: 42, battery_max: 100, groups: [] };
    const out = normalizeConsolePayload(flat, 'power');
    // Keyed by the FINE ids a power console's readers actually use — the ids
    // `buildSystemStationConsoleState` would key a keyed payload by.
    expect(out.systems['power-reactor']).toBe(flat);
    expect(out.systems['power-battery']).toBe(flat);
    // The console-family NAME is NOT a key — no shipped reader queries it.
    expect(out.systems.power).toBeUndefined();
    // Backward-compatible: existing consoles reading top-level fields
    // directly keep working unchanged.
    expect(out.battery_charge).toBe(42);
    expect(out.battery_max).toBe(100);
    expect(out.groups).toEqual([]);
  });

  it('for a family whose fine id is its own name (captain, sensors, ...), that id is still keyed', () => {
    const flat = { red_alert: true, view_direction: 'Fore' };
    const out = normalizeConsolePayload(flat, 'captain');
    // 'captain' here is a FINE id (consoleForSystemId('captain') === 'captain'),
    // not a family alias — the destroyer/courier captain readers query it.
    expect(out.systems.captain).toBe(flat);
    expect(out.systems.viewscreen).toBe(flat);
    expect(out.systems['red-alert']).toBe(flat);
    expect(out.red_alert).toBe(true);
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

  it('falls back to keying under the family name for an unknown family (never worse than pre-fix)', () => {
    const flat = { x: 1 };
    const out = normalizeConsolePayload(flat, 'not-a-real-family');
    expect(out.systems['not-a-real-family']).toBe(flat);
  });

  it('passes through null/non-object payloads untouched', () => {
    expect(normalizeConsolePayload(null, 'power')).toBeNull();
    expect(normalizeConsolePayload(undefined, 'power')).toBeUndefined();
  });

  it('#925 regression: a systemView reader resolves the same data whether the wire payload was flat or keyed', () => {
    // Same logical payload, two wire shapes. The assertions below use ONLY the
    // FINE SYSTEM IDS a shipped power console's readers pass to systemView
    // (e.g. gui/stations/engineering-console.js:
    //   systemView(s, 'power-reactor', 'power-battery')) — NOT the family name,
    // which no shipped console ever queries. That is the real reader contract:
    // this test fails against a family-name-keyed normalisation (the pre-fix
    // bug — systems['power'] is invisible to systemView('power-reactor', …)),
    // and passes once the flat payload is keyed by fine id.
    const view = { battery_charge: 42, battery_max: 100, groups: [] };
    const flatWire = { battery_charge: 42, battery_max: 100, groups: [] };
    const keyedWire = { systems: { 'power-reactor': view } };

    const normalizedFlat = normalizeConsolePayload(flatWire, 'power');
    const normalizedKeyed = normalizeConsolePayload(keyedWire, 'power');

    // Fine ids only — the ids the console's render function actually uses.
    expect(systemView(normalizedFlat, 'power-reactor', 'power-battery')).toEqual(flatWire);
    expect(systemView(normalizedKeyed, 'power-reactor', 'power-battery')).toEqual(view);
    // The two shapes resolve to equal data for the same reader call.
    expect(systemView(normalizedFlat, 'power-reactor', 'power-battery'))
      .toEqual(systemView(normalizedKeyed, 'power-reactor', 'power-battery'));

    // And the same holds for the OTHER three families whose fine id differs
    // from the family name — the ones the family-name wrap silently blanked.
    const shieldsFlat = normalizeConsolePayload({ arc_charges: [1, 2] }, 'shields');
    expect(systemView(shieldsFlat, 'shields-system')).toEqual({ arc_charges: [1, 2] });
    const helmFlat = normalizeConsolePayload({ throttle: 0.5 }, 'helm');
    expect(systemView(helmFlat, 'helm-thrust', 'helm-joystick', 'helm-steering')).toEqual({ throttle: 0.5 });
    const tacticalFlat = normalizeConsolePayload({ banks: [] }, 'tactical');
    expect(systemView(tacticalFlat, 'tactical-radar', 'phaser-control')).toEqual({ banks: [] });

    // Without normalisation, the flat wire shape is exactly the historic
    // blank-console symptom: no `.systems`, so every candidate id misses.
    expect(systemView(flatWire, 'power-reactor', 'power-battery')).toEqual({});
  });
});
