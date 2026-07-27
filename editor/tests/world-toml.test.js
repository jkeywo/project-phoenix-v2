import { describe, it, expect } from 'vitest';
import { readFileSync } from 'fs';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';
import { parseWorldToml, stringifyWorldToml, validateWorldToml } from '../world-toml.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '../..');

function readWorld(name) {
  return readFileSync(resolve(projectRoot, 'assets/worlds', name), 'utf-8');
}

describe('world-toml', () => {
  describe('parseWorldToml', () => {
    it('parses a basic world TOML string', () => {
      const toml = '[global]\nseed = 42\n\n[anchors]\ntest = [1.0, 0.0, 2.0]\n';
      const result = parseWorldToml(toml);
      expect(result.global.seed).toBe(42);
      expect(result.anchors.test).toEqual([1.0, 0.0, 2.0]);
    });

    it('parses default.toml', () => {
      const text = readWorld('default.toml');
      const result = parseWorldToml(text);
      expect(result.global.seed).toBe(42);
      expect(result.anchors.starbase_alpha).toEqual([1000.0, 0.0, 0.0]);
      expect(Array.isArray(result.entity)).toBe(true);
      expect(result.entity.length).toBeGreaterThanOrEqual(5);
      expect(Array.isArray(result.trigger)).toBe(true);
      expect(Array.isArray(result.comms)).toBe(true);
    });

    it('parses patrol.toml', () => {
      const text = readWorld('patrol.toml');
      const result = parseWorldToml(text);
      expect(result.global.seed).toBe(42);
      expect(result.anchors.patrol_alpha).toEqual([600.0, 0.0, -600.0]);
      expect(result.entity.length).toBe(4);
    });

    it('throws on invalid TOML', () => {
      expect(() => parseWorldToml('not valid toml ===')).toThrow();
    });
  });

  describe('stringifyWorldToml', () => {
    it('serializes a parsed world back to string', () => {
      const obj = {
        global: { seed: 42 },
        anchors: { test: [1.0, 0.0, 2.0] },
      };
      const result = stringifyWorldToml(obj);
      expect(result).toBeTruthy();
      expect(typeof result).toBe('string');
    });

    it('produces parseable TOML', () => {
      const obj = {
        global: { seed: 99 },
        anchors: { a: [0.0, 0.0, 0.0] },
      };
      const result = stringifyWorldToml(obj);
      const reparsed = parseWorldToml(result);
      expect(reparsed.global.seed).toBe(99);
      expect(reparsed.anchors.a).toEqual([0.0, 0.0, 0.0]);
    });
  });

  describe('validateWorldToml', () => {
    it('returns valid for a correct world object', () => {
      const obj = {
        global: { seed: 42 },
        anchors: { a: [0.0, 0.0, 0.0] },
      };
      const result = validateWorldToml(obj);
      expect(result.valid).toBe(true);
    });

    it('returns invalid for missing global', () => {
      const result = validateWorldToml({ anchors: {} });
      expect(result.valid).toBe(false);
      expect(result.errors.length).toBeGreaterThan(0);
    });

    it('returns invalid for non-object', () => {
      const result = validateWorldToml(null);
      expect(result.valid).toBe(false);
    });
  });

  describe('extra_worlds field', () => {
    it('parses extra_worlds as an array of strings', () => {
      const toml = '[global]\nseed = 1\n[anchors]\nextra_worlds = ["assets/worlds/patrol.toml", "assets/worlds/side.toml"]\n';
      // extra_worlds at top level, not inside anchors
      const toml2 = '[global]\nseed = 1\n[anchors]\na = [0.0, 0.0, 0.0]\nextra_worlds = ["assets/worlds/patrol.toml"]\n';
      const result = parseWorldToml(toml2);
      // extra_worlds inside anchors section is not the right place — test top-level
      const toml3 = 'extra_worlds = ["assets/worlds/patrol.toml"]\n[global]\nseed = 1\n[anchors]\na = [0.0,0.0,0.0]\n';
      const result3 = parseWorldToml(toml3);
      expect(Array.isArray(result3.extra_worlds)).toBe(true);
      expect(result3.extra_worlds).toEqual(['assets/worlds/patrol.toml']);
    });

    it('extra_worlds round-trips through stringify → parse', () => {
      const obj = {
        extra_worlds: ['assets/worlds/patrol.toml', 'assets/worlds/side.toml'],
        global: { seed: 42 },
        anchors: { a: [0.0, 0.0, 0.0] },
      };
      const serialized = stringifyWorldToml(obj);
      const reparsed = parseWorldToml(serialized);
      expect(reparsed.extra_worlds).toEqual(obj.extra_worlds);
    });

    it('world without extra_worlds has no extra_worlds key', () => {
      const text = readWorld('patrol.toml');
      const parsed = parseWorldToml(text);
      expect(parsed.extra_worlds).toBeUndefined();
    });
  });

  describe('load_world / unload_world trigger actions', () => {
    it('parses a load_world action with path', () => {
      const toml = `
[global]
seed = 1
[anchors]
a = [0.0, 0.0, 0.0]
[[entity]]
template_path = "assets/entities/star_sun.toml"
position = [0.0, 0.0, 0.0]
[[trigger]]
condition = "on_destroyed"
entity = "raider_alpha"
  [[trigger.action]]
  type = "load_world"
  path = "assets/worlds/patrol.toml"
`;
      const result = parseWorldToml(toml);
      expect(result.trigger).toHaveLength(1);
      expect(result.trigger[0].action).toHaveLength(1);
      const action = result.trigger[0].action[0];
      expect(action.type).toBe('load_world');
      expect(action.path).toBe('assets/worlds/patrol.toml');
    });

    it('parses an unload_world action with path', () => {
      const toml = `
[global]
seed = 1
[anchors]
a = [0.0, 0.0, 0.0]
[[entity]]
template_path = "assets/entities/star_sun.toml"
position = [0.0, 0.0, 0.0]
[[trigger]]
condition = "on_timer"
entity = "raider_alpha"
  [[trigger.action]]
  type = "unload_world"
  path = "assets/worlds/patrol.toml"
`;
      const result = parseWorldToml(toml);
      const action = result.trigger[0].action[0];
      expect(action.type).toBe('unload_world');
      expect(action.path).toBe('assets/worlds/patrol.toml');
    });

    it('load_world and unload_world actions round-trip', () => {
      const obj = {
        global: { seed: 1 },
        anchors: { a: [0.0, 0.0, 0.0] },
        entity: [{ template_path: 'assets/entities/star_sun.toml', transform: { position: [0.0, 0.0, 0.0] } }],
        trigger: [
          {
            condition: 'on_destroyed',
            entity: 'raider_alpha',
            action: [{ type: 'load_world', path: 'assets/worlds/patrol.toml' }],
          },
          {
            condition: 'on_timer',
            entity: 'raider_alpha',
            action: [{ type: 'unload_world', path: 'assets/worlds/patrol.toml' }],
          },
        ],
      };
      const serialized = stringifyWorldToml(obj);
      const reparsed = parseWorldToml(serialized);
      expect(reparsed.trigger[0].action[0].type).toBe('load_world');
      expect(reparsed.trigger[0].action[0].path).toBe('assets/worlds/patrol.toml');
      expect(reparsed.trigger[1].action[0].type).toBe('unload_world');
      expect(reparsed.trigger[1].action[0].path).toBe('assets/worlds/patrol.toml');
    });
  });

  describe('round-trip shipped worlds', () => {
    it('default.toml survives parse → stringify → parse with same structure', () => {
      const originalText = readWorld('default.toml');
      const parsed = parseWorldToml(originalText);
      const serialized = stringifyWorldToml(parsed);
      const reparsed = parseWorldToml(serialized);

      expect(reparsed.global.seed).toEqual(parsed.global.seed);
      expect(reparsed.anchors).toEqual(parsed.anchors);
      expect(reparsed.entity.length).toEqual(parsed.entity.length);
      expect(reparsed.trigger.length).toEqual(parsed.trigger.length);
      expect(reparsed.comms.length).toEqual(parsed.comms.length);

      for (let i = 0; i < parsed.entity.length; i++) {
        expect(reparsed.entity[i].template_path).toBe(parsed.entity[i].template_path);
      }
    });

    it('patrol.toml survives parse → stringify → parse with same structure', () => {
      const originalText = readWorld('patrol.toml');
      const parsed = parseWorldToml(originalText);
      const serialized = stringifyWorldToml(parsed);
      const reparsed = parseWorldToml(serialized);

      expect(reparsed.global.seed).toEqual(parsed.global.seed);
      expect(reparsed.anchors).toEqual(parsed.anchors);
      expect(reparsed.entity.length).toEqual(parsed.entity.length);
    });
  });

  describe('spawn override persistence (Slice 3)', () => {
    // Slice 3 introduces `[entity.override]` as a per-spawn convention:
    // the editor mutates `layer.toml.entity[i].override` in place and
    // relies on smol-toml.stringify to round-trip it.  This test pins
    // that contract — if the writer ever drops unknown keys, fix it
    // before merging.
    it('round-trips a primitive override on a spawn', () => {
      const world = {
        global: { seed: 7 },
        anchors: { spot: [0, 0, 0] },
        entity: [{
          template_path: 'assets/entities/ship_harrow_patrol.toml',
          name: 'raider_alpha',
          transform: { anchor: 'spot' },
          override: { hull: { max: 80 } },
        }],
      };
      const toml = stringifyWorldToml(world);
      const reparsed = parseWorldToml(toml);
      expect(reparsed.entity[0].override).toEqual({ hull: { max: 80 } });
      expect(reparsed.entity[0].name).toBe('raider_alpha');
    });

    it('round-trips an array-valued override (REPLACE merge)', () => {
      const world = {
        global: { seed: 1 },
        anchors: { a: [0, 0, 0] },
        entity: [{
          template_path: 'assets/entities/asteroid_large.toml',
          name: 'big_rock',
          override: { radar_appearance: { colour: [0.9, 0.1, 0.1] } },
        }],
      };
      const toml = stringifyWorldToml(world);
      const reparsed = parseWorldToml(toml);
      expect(reparsed.entity[0].override.radar_appearance.colour)
        .toEqual([0.9, 0.1, 0.1]);
    });

    it('a spawn without override has no override key after round-trip', () => {
      const world = {
        global: { seed: 1 },
        anchors: { a: [0, 0, 0] },
        entity: [{
          template_path: 'assets/entities/asteroid_large.toml',
          name: 'plain_rock',
        }],
      };
      const reparsed = parseWorldToml(stringifyWorldToml(world));
      expect('override' in reparsed.entity[0]).toBe(false);
    });
  });
});
