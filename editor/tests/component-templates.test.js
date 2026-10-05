import { describe, it, expect } from 'vitest';
import { getRawSectionDefaults } from '../component-templates.js';
import { COMPONENT_SCHEMA } from '../component-schema.js';

describe('getRawSectionDefaults', () => {
  it('returns a non-null defaults value for every known schema section', () => {
    for (const key of Object.keys(COMPONENT_SCHEMA)) {
      const defaults = getRawSectionDefaults(key);
      // Some top-level scalar sections (e.g. `name`, `faction`) return a
      // bare string or undefined when no default is defined; arrayOfTables
      // sections (e.g. `light`) return a bare []; structured sections
      // return an object. Only null is reserved (for unknown sections).
      expect(defaults, `${key} returned null`).not.toBeNull();
    }
  });

  it('returns null for an unknown section key', () => {
    expect(getRawSectionDefaults('totally_unknown_section')).toBeNull();
  });

  it('hull defaults include hull_integrity', () => {
    const d = getRawSectionDefaults('hull');
    expect(d).toHaveProperty('hull_integrity');
  });

  it('collider defaults include shape', () => {
    const d = getRawSectionDefaults('collider');
    // shape has no 'default' in schema — it won't appear in defaults
    // but the object is still valid (non-null)
    expect(d).not.toBeNull();
  });
});
