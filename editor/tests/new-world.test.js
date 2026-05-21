import { describe, it, expect } from 'vitest';
import { parseWorldToml, stringifyWorldToml, validateWorldToml } from '../world-toml.js';
import {
  createNewWorldContent,
  getDefaultNewWorldPath,
  validateNewWorldPath,
  prepareNewWorld,
} from '../new-world.js';

describe('new-world', () => {
  describe('createNewWorldContent', () => {
    it('returns valid TOML with [global] and seed = 42', () => {
      const content = createNewWorldContent();
      const parsed = parseWorldToml(content);
      expect(parsed.global.seed).toBe(42);
    });

    it('accepts through validateWorldToml', () => {
      const content = createNewWorldContent();
      const parsed = parseWorldToml(content);
      const result = validateWorldToml(parsed);
      expect(result.valid).toBe(true);
    });

    it('round-trips through parseWorldToml -> stringifyWorldToml -> parseWorldToml', () => {
      const content = createNewWorldContent();
      const parsed = parseWorldToml(content);
      const serialised = stringifyWorldToml(parsed);
      const reparsed = parseWorldToml(serialised);
      expect(reparsed.global.seed).toBe(42);
    });
  });

  describe('validateNewWorldPath', () => {
    it('accepts a non-conflicting path under assets/worlds/', () => {
      const existing = ['assets/worlds/default.toml', 'assets/worlds/patrol.toml'];
      const result = validateNewWorldPath('assets/worlds/my_world.toml', existing);
      expect(result.ok).toBe(true);
    });

    it('rejects a path that matches an existing file', () => {
      const existing = ['assets/worlds/default.toml'];
      const result = validateNewWorldPath('assets/worlds/default.toml', existing);
      expect(result.ok).toBe(false);
      expect(result.error).toBeTruthy();
    });

    it('rejects paths outside assets/worlds/', () => {
      const result = validateNewWorldPath('assets/entities/my_entity.toml', []);
      expect(result.ok).toBe(false);
      expect(result.error).toContain('assets/worlds/');
    });
  });

  describe('prepareNewWorld', () => {
    it('returns ok with content and parsedContent', () => {
      const result = prepareNewWorld('assets/worlds/new_world.toml');
      expect(result.ok).toBe(true);
      expect(typeof result.content).toBe('string');
      expect(result.parsedContent).toBeTruthy();
      expect(result.parsedContent.global.seed).toBe(42);
    });
  });

  describe('getDefaultNewWorldPath', () => {
    it('returns assets/worlds/new_world.toml', () => {
      expect(getDefaultNewWorldPath()).toBe('assets/worlds/new_world.toml');
    });
  });
});
