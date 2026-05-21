/**
 * validation-fixtures.test.js — Round-trip the canonical world fixtures
 * (assets/worlds/default.toml and assets/worlds/patrol.toml) through
 * smol-toml parse + validateFile and assert they are clean.
 *
 * Also tests a synthesised broken world to confirm the composed
 * validator surfaces:
 *   - action-schema violations (`trigger[i].action`)
 *   - cross-reference failures (`trigger[i].entity`)
 */
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { parse } from 'smol-toml';
import { validateFile } from '../validation.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const repoRoot = resolve(__dirname, '..', '..');

function loadWorld(relPath) {
  const text = readFileSync(resolve(repoRoot, relPath), 'utf8');
  return parse(text);
}

describe('validation against shipped world fixtures', () => {
  it('assets/worlds/default.toml validates clean', () => {
    const parsed = loadWorld('assets/worlds/default.toml');
    const results = validateFile('assets/worlds/default.toml', parsed);
    // Surface the actual messages on failure so the diff is debuggable.
    expect(results).toEqual([]);
  });

  it('assets/worlds/patrol.toml validates clean', () => {
    const parsed = loadWorld('assets/worlds/patrol.toml');
    const results = validateFile('assets/worlds/patrol.toml', parsed);
    expect(results).toEqual([]);
  });
});

describe('validation surfaces composed errors', () => {
  it('flags an unknown trigger.entity reference', () => {
    const world = {
      global: { seed: 42 },
      anchors: { start: [0, 0, 0] },
      entity: [{ name: 'real_target' }],
      trigger: [
        {
          condition: 'on_destroyed',
          entity: 'phantom_target',
          action: [{ type: 'add_objective', id: 'x', text: 'y' }],
        },
      ],
    };
    const results = validateFile('assets/worlds/broken.toml', world);
    expect(results.length).toBeGreaterThan(0);
    expect(results.some(r => /unknown entity "phantom_target"/.test(r.message))).toBe(true);
  });

  it('flags a malformed action-schema entry', () => {
    const world = {
      global: { seed: 42 },
      anchors: { start: [0, 0, 0] },
      entity: [{ name: 'real_target' }],
      trigger: [
        {
          condition: 'on_destroyed',
          entity: 'real_target',
          // Missing required `id` and `text` for add_objective.
          action: [{ type: 'add_objective' }],
        },
      ],
    };
    const results = validateFile('assets/worlds/broken.toml', world);
    expect(results.some(r => r.path.startsWith('trigger[0].action'))).toBe(true);
    expect(results.some(r => /missing required field/.test(r.message))).toBe(true);
  });

  it('flags an apply_modifier action with an invalid slot', () => {
    const world = {
      global: { seed: 42 },
      anchors: { start: [0, 0, 0] },
      entity: [{ name: 'real_target' }],
      trigger: [
        {
          condition: 'on_attacked',
          entity: 'real_target',
          action: [{
            type: 'apply_modifier',
            entity: 'real_target',
            tag: 'helm_console',
            slot: 'NotAValidSlot',
            bonus: 0.5,
          }],
        },
      ],
    };
    const results = validateFile('assets/worlds/broken.toml', world);
    expect(results.some(r => /invalid value "NotAValidSlot"/.test(r.message))).toBe(true);
  });

  it('flags an unknown reference inside comms.response.action target_entity', () => {
    const world = {
      global: { seed: 42 },
      anchors: { start: [0, 0, 0] },
      entity: [{ name: 'station_a' }],
      comms: [
        {
          from: 'station_a',
          trigger: 'on_hailed',
          entity: 'station_a',
          message: 'Hello.',
          response: [
            {
              text: 'Hi.',
              action: [{ type: 'set_ai_state', entity: 'ghost_entity', state: 'patrol' }],
            },
          ],
        },
      ],
    };
    const results = validateFile('assets/worlds/broken.toml', world);
    expect(results.some(r => /unknown entity "ghost_entity"/.test(r.message))).toBe(true);
  });

  it('treats comms.from as free-form (no error if it is not an entity)', () => {
    const world = {
      global: { seed: 42 },
      anchors: { start: [0, 0, 0] },
      entity: [{ name: 'station_a' }],
      comms: [
        {
          from: 'Some Display Name Not An Entity',
          trigger: 'on_hailed',
          entity: 'station_a',
          message: 'Hi.',
        },
      ],
    };
    const results = validateFile('assets/worlds/broken.toml', world);
    // No reference error should be raised for the `from` field.
    expect(results.filter(r => r.message.includes('Some Display Name'))).toEqual([]);
  });
});
