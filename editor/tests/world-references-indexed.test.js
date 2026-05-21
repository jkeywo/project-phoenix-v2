import { describe, it, expect } from 'vitest';
import { validateWorldReferencesIndexed } from '../world-references-indexed.js';

describe('validateWorldReferencesIndexed', () => {
  it('returns empty for a world with no references', () => {
    expect(validateWorldReferencesIndexed({})).toEqual([]);
    expect(validateWorldReferencesIndexed(null)).toEqual([]);
  });

  it('emits an indexed path for an unknown trigger.entity reference', () => {
    const world = {
      entity: [{ name: 'real' }],
      trigger: [
        { condition: 'on_destroyed', entity: 'real' },          // OK
        { condition: 'on_destroyed', entity: 'phantom' },        // bad
      ],
    };
    const out = validateWorldReferencesIndexed(world);
    expect(out.length).toBe(1);
    expect(out[0].path).toBe('trigger[1].entity');
    expect(out[0].severity).toBe('error');
    expect(out[0].message).toMatch(/phantom/);
  });

  it('emits an indexed path for an unknown trigger.action.entity reference', () => {
    const world = {
      entity: [{ name: 'real' }],
      trigger: [
        {
          condition: 'on_destroyed',
          entity: 'real',
          action: [
            { type: 'set_ai_state', entity: 'real', state: 'patrol' },
            { type: 'set_ai_state', entity: 'phantom', state: 'patrol' },
          ],
        },
      ],
    };
    const out = validateWorldReferencesIndexed(world);
    expect(out.length).toBe(1);
    expect(out[0].path).toBe('trigger[0].action[1].entity');
  });

  it('emits an indexed path for an unknown comms.response.action.entity reference', () => {
    const world = {
      entity: [{ name: 'station_a' }],
      comms: [
        {
          from: 'station_a',
          trigger: 'on_hailed',
          entity: 'station_a',
          response: [
            { text: 'ok', action: [{ type: 'set_ai_state', entity: 'station_a', state: 'idle' }] },
            { text: 'bad', action: [{ type: 'set_ai_state', entity: 'ghost', state: 'idle' }] },
          ],
        },
      ],
    };
    const out = validateWorldReferencesIndexed(world);
    expect(out.length).toBe(1);
    expect(out[0].path).toBe('comms[0].response[1].action[0].entity');
  });

  it('handles target_entity field on actions', () => {
    const world = {
      entity: [{ name: 'real' }],
      trigger: [
        {
          condition: 'on_attacked',
          entity: 'real',
          action: [{ type: 'apply_modifier', target_entity: 'phantom', slot: 'helm_console_speed', bonus: 0.1 }],
        },
      ],
    };
    const out = validateWorldReferencesIndexed(world);
    expect(out.some((r) => r.path === 'trigger[0].action[0].target_entity')).toBe(true);
  });

  it('does NOT fire for known entities', () => {
    const world = {
      entity: [{ name: 'foo' }],
      trigger: [{ condition: 'on_destroyed', entity: 'foo' }],
    };
    expect(validateWorldReferencesIndexed(world)).toEqual([]);
  });

  it('does NOT validate comms.from (mirrors world-references.js)', () => {
    const world = {
      entity: [{ name: 'station_a' }],
      comms: [
        { from: 'Some Display Name', trigger: 'on_hailed', entity: 'station_a' },
      ],
    };
    const out = validateWorldReferencesIndexed(world);
    expect(out.filter((r) => r.message.includes('Some Display Name'))).toEqual([]);
  });

  it('flags a comms[i].entity unknown reference with indexed path', () => {
    const world = {
      entity: [{ name: 'real' }],
      comms: [
        { from: 'real', trigger: 'on_hailed', entity: 'phantom' },
      ],
    };
    const out = validateWorldReferencesIndexed(world);
    expect(out.some((r) => r.path === 'comms[0].entity')).toBe(true);
  });
});
