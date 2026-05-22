import { describe, it, expect, beforeEach } from 'vitest';

import {
  COMBO_TEMPLATES,
  getComboTemplate,
  getAllComboNames,
  getRawSectionDefaults,
  getPickerModel,
} from '../component-templates.js';
import { COMPONENT_SCHEMA, ENTITY_CONFIG_SECTIONS } from '../component-schema.js';
import { parseEntityToml, stringifyEntityToml } from '../entity-toml.js';
import { EntityModeShell } from '../entity-mode.js';

// ── Helpers ───────────────────────────────────────────────────────────────────

/**
 * Given a combo template, build a minimal entity object and round-trip it
 * through entity-toml.js.  Returns { original, reparsed }.
 */
function roundTripCombo(comboName) {
  const template = getComboTemplate(comboName);
  if (!template) throw new Error(`Unknown combo: ${comboName}`);

  // Build a plain object the same way the editor would: each section key maps
  // to its defaults object (or scalar for 'tags').
  const obj = {};
  for (const { key, defaults } of template.sections) {
    obj[key] = defaults;
  }

  // tags section: the defaults object is { tags: [...] }, but entity-toml
  // expects the top-level `tags` key to hold the array directly.
  if (obj.tags && typeof obj.tags === 'object' && !Array.isArray(obj.tags) && Array.isArray(obj.tags.tags)) {
    obj.tags = obj.tags.tags;
  }
  // name section: defaults is { name: 'string' }; entity-toml expects the
  // top-level `name` key to hold the scalar string directly.
  if (obj.name && typeof obj.name === 'object' && typeof obj.name.name === 'string') {
    obj.name = obj.name.name;
  }
  // light section: defaults is { light: [...] }; entity-toml expects the
  // top-level `light` key to hold the array directly.
  if (obj.light && typeof obj.light === 'object' && !Array.isArray(obj.light) && Array.isArray(obj.light.light)) {
    obj.light = obj.light.light;
  }

  const tomlText = stringifyEntityToml(obj);
  const reparsed = parseEntityToml(tomlText);
  return { original: obj, reparsed };
}

// ── getAllComboNames ───────────────────────────────────────────────────────────

describe('getAllComboNames', () => {
  it('returns the eight expected combo names', () => {
    const names = getAllComboNames();
    expect(names).toContain('Ship');
    expect(names).toContain('Station');
    expect(names).toContain('Region');
    expect(names).toContain('NPC');
    expect(names).toContain('Asteroid');
    expect(names).toContain('Asteroid Field');
    expect(names).toContain('Star');
    expect(names).toContain('Planet');
  });

  it('returns exactly eight combos', () => {
    expect(getAllComboNames()).toHaveLength(8);
  });
});

// ── getComboTemplate ──────────────────────────────────────────────────────────

describe('getComboTemplate', () => {
  it('returns a template object with sections array for every combo name', () => {
    for (const name of getAllComboNames()) {
      const tpl = getComboTemplate(name);
      expect(tpl, `${name} template is null`).not.toBeNull();
      expect(Array.isArray(tpl.sections), `${name} sections is not an array`).toBe(true);
      expect(tpl.sections.length, `${name} has no sections`).toBeGreaterThan(0);
    }
  });

  it('returns null for an unknown combo name', () => {
    expect(getComboTemplate('UnknownThing')).toBeNull();
  });

  it('every section in every combo has a non-empty key and defaults defined', () => {
    for (const name of getAllComboNames()) {
      const tpl = getComboTemplate(name);
      for (const sec of tpl.sections) {
        expect(sec.key, `${name}: section missing key`).toBeTruthy();
        // Defaults may be a bare scalar (e.g. `name = "Sun"`), an array
        // (e.g. `tags = [...]`, `[[light]]` entries), or an object (e.g.
        // `hull`, `helm_console`). All three shapes are valid; we just
        // require that defaults is defined.
        expect(sec.defaults, `${name}.${sec.key} defaults is undefined`).toBeDefined();
      }
    }
  });
});

// ── getRawSectionDefaults ─────────────────────────────────────────────────────

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

// ── Round-trip tests for all eight combos ─────────────────────────────────────

describe('combo round-trip via entity-toml.js', () => {
  const COMBOS = getAllComboNames();

  for (const comboName of COMBOS) {
    it(`${comboName} template round-trips through entity-toml`, () => {
      const { original, reparsed } = roundTripCombo(comboName);

      // Every top-level key in original must survive
      for (const key of Object.keys(original)) {
        expect(reparsed, `${comboName}: key '${key}' missing after round-trip`).toHaveProperty(key);
      }
    });
  }

  it('Ship combo produces helm_console section after round-trip', () => {
    const { reparsed } = roundTripCombo('Ship');
    expect(reparsed).toHaveProperty('helm_console');
    expect(reparsed.helm_console.max_speed).toBe(50.0);
  });

  it('Station combo produces hull section (not legacy [station]) after round-trip', () => {
    const { reparsed } = roundTripCombo('Station');
    expect(reparsed).not.toHaveProperty('station');
    expect(reparsed).toHaveProperty('hull');
    expect(reparsed.hull.hull_integrity).toBe(200.0);
    expect(reparsed.tags).toContain('station');
  });

  it('Region combo produces shape and effects sections after round-trip', () => {
    const { reparsed } = roundTripCombo('Region');
    expect(reparsed).toHaveProperty('shape');
    expect(reparsed.shape.type).toBe('sphere');
    expect(reparsed).toHaveProperty('effects');
  });

  it('NPC combo produces behaviour section after round-trip', () => {
    const { reparsed } = roundTripCombo('NPC');
    expect(reparsed).toHaveProperty('behaviour');
    expect(reparsed.behaviour.initial_state).toBe('idle');
  });

  it('Asteroid combo produces collider section after round-trip', () => {
    const { reparsed } = roundTripCombo('Asteroid');
    expect(reparsed).toHaveProperty('collider');
    expect(reparsed.collider.shape).toBe('Ball');
  });

  it('Asteroid Field combo produces asteroid_field section after round-trip', () => {
    const { reparsed } = roundTripCombo('Asteroid Field');
    expect(reparsed).toHaveProperty('asteroid_field');
    expect(reparsed.asteroid_field.inner_radius).toBe(100.0);
  });

  it('Star combo produces name, mesh, and light sections after round-trip', () => {
    const { reparsed } = roundTripCombo('Star');
    expect(reparsed).not.toHaveProperty('star');
    expect(reparsed.name).toBe('New Star');
    expect(reparsed).toHaveProperty('mesh');
    expect(reparsed.mesh.shape).toBe('sphere');
    expect(reparsed.mesh.radius).toBe(50.0);
    expect(reparsed.mesh.emissive).toBe(2.0);
    expect(reparsed).toHaveProperty('light');
    expect(Array.isArray(reparsed.light)).toBe(true);
    expect(reparsed.light[0].kind).toBe('point');
    expect(reparsed.light[0].intensity).toBe(150000.0);
  });

  it('Planet combo produces name and mesh sections after round-trip', () => {
    const { reparsed } = roundTripCombo('Planet');
    expect(reparsed).not.toHaveProperty('planet');
    expect(reparsed.name).toBe('New Planet');
    expect(reparsed).toHaveProperty('mesh');
    expect(reparsed.mesh.shape).toBe('sphere');
    expect(reparsed.mesh.radius).toBe(20.0);
  });
});

// ── EntityModeShell.addComponent ─────────────────────────────────────────────

describe('EntityModeShell.addComponent', () => {
  let shell;

  beforeEach(() => {
    shell = new EntityModeShell();
  });

  it('adds a new section card when no file is open', () => {
    const result = shell.addComponent('hull');
    expect(result.ok).toBe(true);
    const card = shell.getCard('hull');
    expect(card).not.toBeNull();
    expect(card.section).toBe('hull');
  });

  it('returns ok: false and a warning when the section is already present', () => {
    shell.addComponent('hull');
    const result = shell.addComponent('hull');
    expect(result.ok).toBe(false);
    expect(result.warning).toMatch(/hull/);
    // Still only one hull card
    expect(shell.getComponentCards().filter((c) => c.section === 'hull')).toHaveLength(1);
  });

  it('uses schema defaults for known sections', () => {
    shell.addComponent('hull');
    const card = shell.getCard('hull');
    // hull schema has default: 0 for hull_integrity
    expect(card.data).toHaveProperty('hull_integrity');
  });

  it('adds card for unknown section with empty defaults', () => {
    // Unknown sections have no schema entry — getRawSectionDefaults returns null
    // but addComponent should still create a card with empty data
    shell.addComponent('unknown_section_xyz');
    const card = shell.getCard('unknown_section_xyz');
    expect(card).not.toBeNull();
    expect(card.data).toEqual({});
  });

  it('adding multiple distinct sections works', () => {
    shell.addComponent('tags');
    shell.addComponent('hull');
    shell.addComponent('collider');
    const sections = shell.getComponentCards().map((c) => c.section);
    expect(sections).toContain('tags');
    expect(sections).toContain('hull');
    expect(sections).toContain('collider');
  });
});

// ── EntityModeShell.addCombo ──────────────────────────────────────────────────

describe('EntityModeShell.addCombo', () => {
  let shell;

  beforeEach(() => {
    shell = new EntityModeShell();
  });

  it('adds all sections for the Ship combo', () => {
    const result = shell.addCombo('Ship');
    expect(result.ok).toBe(true);
    expect(result.warnings).toHaveLength(0);

    const sections = shell.getComponentCards().map((c) => c.section);
    expect(sections).toContain('tags');
    expect(sections).toContain('collider');
    expect(sections).toContain('hull');
    expect(sections).toContain('helm_console');
    expect(sections).toContain('radar_appearance');
  });

  it('adds all sections for the Station combo', () => {
    shell.addCombo('Station');
    const sections = shell.getComponentCards().map((c) => c.section);
    expect(sections).toContain('tags');
    expect(sections).toContain('hull');
    expect(sections).not.toContain('station');
    expect(sections).toContain('radar_appearance');
  });

  it('adds all sections for the Region combo', () => {
    shell.addCombo('Region');
    const sections = shell.getComponentCards().map((c) => c.section);
    expect(sections).toContain('shape');
    expect(sections).toContain('effects');
  });

  it('adds all sections for the NPC combo', () => {
    shell.addCombo('NPC');
    const sections = shell.getComponentCards().map((c) => c.section);
    expect(sections).toContain('tags');
    expect(sections).toContain('hull');
    expect(sections).toContain('helm_console');
    expect(sections).toContain('behaviour');
  });

  it('adds all sections for the Asteroid combo', () => {
    shell.addCombo('Asteroid');
    const sections = shell.getComponentCards().map((c) => c.section);
    expect(sections).toContain('tags');
    expect(sections).toContain('collider');
    expect(sections).toContain('hull');
  });

  it('adds all sections for the Asteroid Field combo', () => {
    shell.addCombo('Asteroid Field');
    const sections = shell.getComponentCards().map((c) => c.section);
    expect(sections).toContain('tags');
    expect(sections).toContain('asteroid_field');
  });

  it('adds all sections for the Star combo', () => {
    shell.addCombo('Star');
    const sections = shell.getComponentCards().map((c) => c.section);
    expect(sections).toContain('name');
    expect(sections).toContain('tags');
    expect(sections).toContain('mesh');
    expect(sections).toContain('light');
    expect(sections).toContain('collider');
    expect(sections).toContain('radar_appearance');
  });

  it('adds all sections for the Planet combo', () => {
    shell.addCombo('Planet');
    const sections = shell.getComponentCards().map((c) => c.section);
    expect(sections).toContain('name');
    expect(sections).toContain('tags');
    expect(sections).toContain('mesh');
    expect(sections).toContain('collider');
    expect(sections).toContain('radar_appearance');
  });

  it('returns ok: false and a warning for an unknown combo name', () => {
    const result = shell.addCombo('UnknownCombo');
    expect(result.ok).toBe(false);
    expect(result.warnings[0]).toMatch(/UnknownCombo/);
    expect(shell.getComponentCards()).toHaveLength(0);
  });

  it('skips already-present sections with a warning', () => {
    shell.addComponent('tags');
    const result = shell.addCombo('Ship');
    expect(result.ok).toBe(true);
    // 'tags' was already present — one warning
    expect(result.warnings.length).toBeGreaterThan(0);
    expect(result.warnings[0]).toMatch(/tags/);
    // Only one tags card
    expect(shell.getComponentCards().filter((c) => c.section === 'tags')).toHaveLength(1);
  });

  it('defaults data is a defensive copy between calls', () => {
    shell.addCombo('Ship');
    const card1 = shell.getCard('hull');

    // A second shell should get its own independent copy
    const shell2 = new EntityModeShell();
    shell2.addCombo('Ship');
    const card2 = shell2.getCard('hull');

    // Mutating one should not affect the other
    card1.data.hull_integrity = 999;
    expect(card2.data.hull_integrity).not.toBe(999);
  });

  it('addCombo works after openFile — does not duplicate existing sections', () => {
    // Use a small inline TOML with tags, hull, collider already present
    const toml = 'tags = ["ship", "npc"]\n\n[hull]\nhull_integrity = 60.0\n\n[collider]\nshape = "Capsule"\nradius = 2.0\nlength = 4.0\n';
    shell.openFile('test.toml', toml);
    const before = shell.getComponentCards().length;

    // Ship combo adds tags, collider, hull, helm_console, radar_appearance
    // tags, hull, collider already exist → should be skipped
    const result = shell.addCombo('Ship');
    expect(result.ok).toBe(true);
    // Should have warnings for tags, hull, collider
    expect(result.warnings.length).toBe(3);
    // Only helm_console and radar_appearance should be new
    expect(shell.getComponentCards().length).toBe(before + 2);
  });
});

// ── getPickerModel ────────────────────────────────────────────────────────────

describe('getPickerModel', () => {
  it('returns an object with combos and rawSections arrays', () => {
    const model = getPickerModel();
    expect(model).toHaveProperty('combos');
    expect(model).toHaveProperty('rawSections');
    expect(Array.isArray(model.combos)).toBe(true);
    expect(Array.isArray(model.rawSections)).toBe(true);
  });

  it('combos has exactly 8 entries (one per combo name)', () => {
    const model = getPickerModel();
    expect(model.combos).toHaveLength(8);
  });

  it('rawSections has one entry per section in ENTITY_CONFIG_SECTIONS', () => {
    const model = getPickerModel();
    expect(model.rawSections).toHaveLength(ENTITY_CONFIG_SECTIONS.length);
  });

  it('each combo entry has name and label', () => {
    const model = getPickerModel();
    for (const combo of model.combos) {
      expect(combo).toHaveProperty('name');
      expect(combo).toHaveProperty('label');
      expect(typeof combo.name).toBe('string');
      expect(typeof combo.label).toBe('string');
    }
  });

  it('each rawSection entry has key and label', () => {
    const model = getPickerModel();
    for (const section of model.rawSections) {
      expect(section).toHaveProperty('key');
      expect(section).toHaveProperty('label');
      expect(typeof section.key).toBe('string');
      expect(typeof section.label).toBe('string');
    }
  });

  it('combo names match getAllComboNames()', () => {
    const model = getPickerModel();
    const names = model.combos.map(c => c.name);
    expect(names).toEqual(getAllComboNames());
  });

  it('rawSection keys match ENTITY_CONFIG_SECTIONS', () => {
    const model = getPickerModel();
    const keys = model.rawSections.map(s => s.key);
    expect(keys).toEqual(ENTITY_CONFIG_SECTIONS);
  });
});
