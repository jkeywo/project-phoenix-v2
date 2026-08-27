import { describe, it, expect } from 'vitest';
import { familyView, normalizeConsolePayload } from '../../gui/console-payload.js';

describe('familyView — authoritative Console Family projection', () => {
  it('selects an arbitrarily named System by projected family', () => {
    const view = { level: 3 };
    const payload = {
      system_ids: ['main-bus-alpha'],
      system_families: { 'main-bus-alpha': 'power' },
      systems: { 'main-bus-alpha': view },
    };
    expect(familyView(payload, 'power')).toBe(view);
  });

  it('uses authored order when multiple Systems share one aggregate view', () => {
    const payload = {
      system_ids: ['starboard-drive', 'port-drive'],
      system_families: { 'starboard-drive': 'helm', 'port-drive': 'helm' },
      systems: { 'port-drive': { side: 'port' }, 'starboard-drive': { side: 'starboard' } },
    };
    expect(familyView(payload, 'helm')).toEqual({ side: 'starboard' });
  });

  it('sorts visiting Systems after authored ids for deterministic selection', () => {
    const payload = {
      system_ids: ['bridge-core'],
      system_families: { 'bridge-core': 'captain', zebra: 'comms', alpha: 'comms' },
      systems: { zebra: { id: 'zebra' }, alpha: { id: 'alpha' }, 'bridge-core': {} },
    };
    expect(familyView(payload, 'comms')).toEqual({ id: 'alpha' });
  });

  it('does not infer a family from an exact legacy id or a prefix', () => {
    const payload = {
      system_ids: ['power-reactor', 'phaser-surprise'],
      system_families: {},
      systems: { 'power-reactor': { level: 3 }, 'phaser-surprise': { ready: true } },
    };
    expect(familyView(payload, 'power')).toEqual({});
    expect(familyView(payload, 'tactical')).toEqual({});
  });

  it('returns an empty view for malformed or absent payload state', () => {
    expect(familyView({}, 'repair')).toEqual({});
    expect(familyView(null, 'repair')).toEqual({});
  });
});

describe('normalizeConsolePayload — actual owned System ids', () => {
  it('mirrors a flat view under projected arbitrary instance ids', () => {
    const flat = {
      battery_charge: 42,
      system_ids: ['reactor-port', 'reserve-cell'],
      system_families: { 'reactor-port': 'power', 'reserve-cell': 'power' },
    };
    const out = normalizeConsolePayload(flat);
    expect(out.systems['reactor-port']).toBe(flat);
    expect(out.systems['reserve-cell']).toBe(flat);
    expect(familyView(out, 'power')).toBe(flat);
    expect(out.battery_charge).toBe(42);
  });

  it('leaves an already keyed payload unchanged', () => {
    const keyed = {
      system_ids: ['reactor-port'],
      system_families: { 'reactor-port': 'power' },
      systems: { 'reactor-port': { level: 3 } },
    };
    expect(normalizeConsolePayload(keyed)).toBe(keyed);
  });

  it('does not invent keys when the authoritative projection is absent', () => {
    const flat = { battery_charge: 42, system_ids: ['power-reactor'] };
    expect(normalizeConsolePayload(flat)).toBe(flat);
    expect(flat.systems).toBeUndefined();
  });

  it('passes through null and non-object payloads', () => {
    expect(normalizeConsolePayload(null)).toBeNull();
    expect(normalizeConsolePayload(undefined)).toBeUndefined();
  });

  it('resolves the same family view for flat and keyed wire shapes', () => {
    const view = { battery_charge: 42 };
    const metadata = {
      system_ids: ['unconventional-reactor-id'],
      system_families: { 'unconventional-reactor-id': 'power' },
    };
    const flat = normalizeConsolePayload({ ...view, ...metadata });
    const keyed = normalizeConsolePayload({
      ...metadata,
      systems: { 'unconventional-reactor-id': view },
    });
    expect(familyView(flat, 'power')).toMatchObject(view);
    expect(familyView(keyed, 'power')).toEqual(view);
  });
});
