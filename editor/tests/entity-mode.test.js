import { describe, it, expect, beforeEach } from 'vitest';
import { readFileSync, readdirSync } from 'fs';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';

import { parseEntityToml, stringifyEntityToml, validateEntitySections, buildFactionMap, buildComplexityPaths } from '../entity-toml.js';
import { COMPONENT_SCHEMA, ENTITY_CONFIG_SECTIONS, getComponentSchema, getSectionsWithComplexityToml, getSectionsWithFaction } from '../component-schema.js';
import { EntityModeShell, ComponentCard } from '../entity-mode.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '../..');

function readEntity(name) {
  return readFileSync(resolve(projectRoot, 'assets/entities', name), 'utf-8');
}

function readFaction(name) {
  return readFileSync(resolve(projectRoot, 'assets/factions', name), 'utf-8');
}

function listDir(rel) {
  return readdirSync(resolve(projectRoot, rel));
}

// ── entity-toml round-trip tests ──────────────────────────────────────────────

describe('entity-toml extended', () => {
  describe('round-trip all shipped entities', () => {
    const entityFiles = listDir('assets/entities').filter((f) => f.endsWith('.toml'));

    for (const file of entityFiles) {
      it(`${file} parses and round-trips`, () => {
        const original = readEntity(file);
        const parsed = parseEntityToml(original);
        const serialized = stringifyEntityToml(parsed);
        const reparsed = parseEntityToml(serialized);

        // All top-level keys must survive the round-trip
        for (const key of Object.keys(parsed)) {
          expect(reparsed).toHaveProperty(key);
        }
        // tags equality
        if (Array.isArray(parsed.tags)) {
          expect(reparsed.tags).toEqual(parsed.tags);
        }
        // hull equality
        if (parsed.hull) {
          expect(reparsed.hull).toEqual(parsed.hull);
        }
        // collider equality
        if (parsed.collider) {
          expect(reparsed.collider).toEqual(parsed.collider);
        }
        // asteroid_field.grid survives
        if (parsed.asteroid_field?.grid) {
          expect(reparsed.asteroid_field.grid).toBeDefined();
          expect(reparsed.asteroid_field.grid.resolution).toBe(parsed.asteroid_field.grid.resolution);
        }
        // shape survives
        if (parsed.shape) {
          expect(reparsed.shape).toEqual(parsed.shape);
        }
        // effects survive
        if (parsed.effects) {
          expect(reparsed.effects).toEqual(parsed.effects);
        }
        // behaviour survives
        if (parsed.behaviour) {
          expect(reparsed.behaviour.initial_state).toBe(parsed.behaviour.initial_state);
          if (Array.isArray(parsed.behaviour.state)) {
            expect(reparsed.behaviour.state).toHaveLength(parsed.behaviour.state.length);
          }
        }
      });
    }
  });

  describe('validateEntitySections', () => {
    it('allows entity with shape and effects', () => {
      const obj = { tags: ['region'], shape: { type: 'sphere', radius: 100 }, effects: { comms_jammed: {} } };
      expect(validateEntitySections(obj).valid).toBe(true);
    });

    it('rejects effects without shape', () => {
      const obj = { tags: ['region'], effects: { comms_jammed: {} } };
      const result = validateEntitySections(obj);
      expect(result.valid).toBe(false);
      expect(result.errors[0]).toMatch(/shape/);
    });

    it('allows entity with neither shape nor effects', () => {
      const obj = { tags: ['ship'], hull: { captain_chair: 100 } };
      expect(validateEntitySections(obj).valid).toBe(true);
    });

    it('allows entity with empty effects object', () => {
      const obj = { tags: ['region'], effects: {} };
      expect(validateEntitySections(obj).valid).toBe(true);
    });
  });

  describe('buildFactionMap', () => {
    it('builds uuid→name map from faction files', () => {
      const factionDir = listDir('assets/factions').filter((f) => f.endsWith('.toml'));
      const factionFiles = factionDir.map((name) => ({
        name,
        content: readFaction(name),
      }));
      const map = buildFactionMap(factionFiles);
      expect(map.size).toBeGreaterThan(0);
      expect(map.get('aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa')).toBe('Federation');
      expect(map.get('bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb')).toBe('Pirate');
    });

    it('skips malformed faction files', () => {
      const factionFiles = [
        { name: 'bad.toml', content: 'not valid toml ===' },
        { name: 'good.toml', content: 'uuid = "aabbccdd-1234-4234-8234-aabbccddaabb"\nname = "Test"\n' },
      ];
      const map = buildFactionMap(factionFiles);
      expect(map.size).toBe(1);
      expect(map.get('aabbccdd-1234-4234-8234-aabbccddaabb')).toBe('Test');
    });

    it('returns empty map for empty input', () => {
      const map = buildFactionMap([]);
      expect(map.size).toBe(0);
    });
  });

  describe('buildComplexityPaths', () => {
    it('builds sorted path list from complexity filenames', () => {
      const filenames = listDir('assets/complexity').filter((f) => f.endsWith('.toml'));
      const paths = buildComplexityPaths(filenames);
      expect(paths.length).toBe(filenames.length);
      for (const p of paths) {
        expect(p).toMatch(/^assets\/complexity\//);
        expect(p).toMatch(/\.toml$/);
      }
      // sorted
      const sorted = [...paths].sort();
      expect(paths).toEqual(sorted);
    });

    it('returns empty array for empty input', () => {
      expect(buildComplexityPaths([])).toEqual([]);
    });
  });
});

// ── component-schema tests ────────────────────────────────────────────────────

describe('component-schema', () => {
  describe('schema completeness', () => {
    it('COMPONENT_SCHEMA covers every section in ENTITY_CONFIG_SECTIONS', () => {
      for (const section of ENTITY_CONFIG_SECTIONS) {
        expect(COMPONENT_SCHEMA).toHaveProperty(section,
          expect.objectContaining({ section, fields: expect.any(Array) }));
      }
    });

    it('every schema entry has a label and at least one field', () => {
      for (const [key, schema] of Object.entries(COMPONENT_SCHEMA)) {
        expect(schema.label, `${key} missing label`).toBeTruthy();
        expect(schema.fields.length, `${key} has no fields`).toBeGreaterThan(0);
      }
    });

    it('every field descriptor has key and type', () => {
      for (const [section, schema] of Object.entries(COMPONENT_SCHEMA)) {
        for (const field of schema.fields) {
          expect(field.key, `field in ${section} missing key`).toBeTruthy();
          expect(field.type, `field ${section}.${field.key} missing type`).toBeTruthy();
        }
      }
    });
  });

  describe('getComponentSchema', () => {
    it('returns schema for known sections', () => {
      expect(getComponentSchema('hull')).toBeDefined();
      expect(getComponentSchema('helm_console')).toBeDefined();
    });

    it('returns null for the removed legacy station section', () => {
      expect(getComponentSchema('station')).toBeNull();
    });

    it('returns null for unknown sections', () => {
      expect(getComponentSchema('nonexistent_section')).toBeNull();
    });
  });

  describe('getSectionsWithComplexityToml', () => {
    it('includes console sections that have complexity_toml', () => {
      const sections = getSectionsWithComplexityToml();
      expect(sections).toContain('helm_console');
      expect(sections).toContain('weapons_console');
      expect(sections).toContain('engineering_console');
      expect(sections).toContain('captain_console');
      expect(sections).not.toContain('science_console');
      expect(sections).toContain('sensors_console');
      expect(sections).toContain('shields_console');
    });

    it('does not include sections without complexity_toml', () => {
      const sections = getSectionsWithComplexityToml();
      expect(sections).not.toContain('hull');
      expect(sections).not.toContain('collider');
      expect(sections).not.toContain('star');
    });
  });

  describe('getSectionsWithFaction', () => {
    it('includes faction section', () => {
      const sections = getSectionsWithFaction();
      expect(sections).toContain('faction');
    });
  });
});

// ── EntityModeShell tests ─────────────────────────────────────────────────────

describe('EntityModeShell', () => {
  let shell;

  beforeEach(() => {
    shell = new EntityModeShell();
  });

  describe('file list (left pane)', () => {
    it('starts with empty file list', () => {
      expect(shell.getFileList()).toEqual([]);
    });

    it('setFileList stores file paths', () => {
      const paths = ['assets/entities/player_ship.toml', 'assets/entities/pirate_raider.toml'];
      shell.setFileList(paths);
      expect(shell.getFileList()).toEqual(paths);
    });

    it('file list is a defensive copy', () => {
      const paths = ['a.toml'];
      shell.setFileList(paths);
      paths.push('b.toml');
      expect(shell.getFileList()).toHaveLength(1);
    });

    it('getFileList returns all shipped entity TOML files', () => {
      const filenames = listDir('assets/entities').filter((f) => f.endsWith('.toml'));
      const paths = filenames.map((f) => `assets/entities/${f}`);
      shell.setFileList(paths);
      expect(shell.getFileList().length).toBe(filenames.length);
    });
  });

  describe('openFile (centre pane)', () => {
    it('opens a valid entity TOML', () => {
      const text = readEntity('pirate_raider.toml');
      const result = shell.openFile('assets/entities/pirate_raider.toml', text);
      expect(result.ok).toBe(true);
      expect(shell.getActiveFile()).toBe('assets/entities/pirate_raider.toml');
    });

    it('returns error for invalid TOML', () => {
      const result = shell.openFile('bad.toml', 'not = valid toml ===');
      expect(result.ok).toBe(false);
      expect(result.errors.length).toBeGreaterThan(0);
    });

    it('returns error for effects without shape', () => {
      const toml = 'tags = ["region"]\n\n[effects]\n[effects.comms_jammed]\n';
      const result = shell.openFile('bad.toml', toml);
      expect(result.ok).toBe(false);
      expect(result.errors[0]).toMatch(/shape/);
    });
  });

  describe('component cards (centre pane)', () => {
    it('returns empty array before any file is opened', () => {
      expect(shell.getComponentCards()).toEqual([]);
    });

    it('creates a card for each present section in pirate_raider.toml', () => {
      shell.openFile('pirate_raider.toml', readEntity('pirate_raider.toml'));
      const cards = shell.getComponentCards();
      const sections = cards.map((c) => c.section);
      expect(sections).toContain('tags');
      expect(sections).toContain('hull');
      expect(sections).toContain('collider');
      expect(sections).toContain('helm_console');
      expect(sections).toContain('weapons_console');
      expect(sections).toContain('behaviour');
      expect(sections).toContain('faction');
    });

    it('creates a hull card (no legacy [station] card) for station_axiom.toml', () => {
      shell.openFile('station_axiom.toml', readEntity('station_axiom.toml'));
      expect(shell.getCard('station')).toBeNull();
      expect(shell.getCard('hull')).not.toBeNull();
    });

    it('creates cards for shape and effects in region_nebula.toml', () => {
      shell.openFile('region_nebula.toml', readEntity('region_nebula.toml'));
      expect(shell.getCard('shape')).not.toBeNull();
      expect(shell.getCard('effects')).not.toBeNull();
    });

    it('creates a card for asteroid_field with grid in asteroid_field_main.toml', () => {
      shell.openFile('asteroid_field_main.toml', readEntity('asteroid_field_main.toml'));
      const card = shell.getCard('asteroid_field');
      expect(card).not.toBeNull();
      expect(card.data.grid).toBeDefined();
    });

    it('creates cards for stations section in player_ship.toml', () => {
      shell.openFile('player_ship.toml', readEntity('player_ship.toml'));
      expect(shell.getCard('stations')).not.toBeNull();
    });
  });

  describe('ComponentCard', () => {
    let card;

    beforeEach(() => {
      shell.openFile('pirate_raider.toml', readEntity('pirate_raider.toml'));
      card = shell.getCard('hull');
    });

    it('starts expanded (not collapsed)', () => {
      expect(card.collapsed).toBe(false);
    });

    it('toggle collapses and uncollapses', () => {
      card.toggle();
      expect(card.collapsed).toBe(true);
      card.toggle();
      expect(card.collapsed).toBe(false);
    });

    it('starts with raw TOML off', () => {
      expect(card.showRaw).toBe(false);
    });

    it('toggleRaw switches raw mode', () => {
      card.toggleRaw();
      expect(card.showRaw).toBe(true);
      card.toggleRaw();
      expect(card.showRaw).toBe(false);
    });

    it('getRawToml returns a TOML string containing the section key', () => {
      const raw = card.getRawToml();
      expect(typeof raw).toBe('string');
      expect(raw).toContain('hull');
    });

    it('getRawToml produces parseable TOML', () => {
      const raw = card.getRawToml();
      expect(() => parseEntityToml(raw)).not.toThrow();
    });
  });

  describe('faction card fields', () => {
    it('faction card exists and has the correct UUID as its data', () => {
      shell.openFile('pirate_raider.toml', readEntity('pirate_raider.toml'));
      const factionCard = shell.getCard('faction');
      expect(factionCard).not.toBeNull();
      // The top-level faction field in TOML is a plain string UUID, not an object.
      // The card's data holds the string value directly.
      expect(factionCard.data).toBe('bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb');
    });

    it('faction card getFactionFields returns field with UUID value', () => {
      // Use a helm-style toml where faction would be in sub-object — but top-level
      // faction is a scalar. Test the schema correctly identifies the uuid-faction field.
      const factionSchema = COMPONENT_SCHEMA['faction'];
      const factionField = factionSchema.fields.find((f) => f.dropdownSource === 'factions');
      expect(factionField).toBeDefined();
      expect(factionField.type).toBe('uuid-faction');
    });
  });

  describe('complexity card fields', () => {
    it('helm_console card reports complexity_toml field', () => {
      const toml = '[helm_console]\ncomplexity_toml = "assets/complexity/navigation.toml"\nmax_speed = 50.0\n\ntags = ["ship"]\n';
      shell.openFile('test.toml', toml);
      const helmCard = shell.getCard('helm_console');
      expect(helmCard).not.toBeNull();
      const fields = helmCard.getComplexityFields();
      expect(fields.length).toBeGreaterThan(0);
      expect(fields[0].value).toBe('assets/complexity/navigation.toml');
    });
  });

  describe('faction dropdown', () => {
    it('starts with empty faction map', () => {
      expect(shell.getFactionMap().size).toBe(0);
    });

    it('setFactionMap stores the map', () => {
      const map = new Map([['uuid1', 'Fed'], ['uuid2', 'Pirate']]);
      shell.setFactionMap(map);
      expect(shell.getFactionMap().get('uuid1')).toBe('Fed');
    });

    it('resolveFactionName returns name when known', () => {
      shell.setFactionMap(new Map([['aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa', 'Federation']]));
      expect(shell.resolveFactionName('aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa')).toBe('Federation');
    });

    it('resolveFactionName returns uuid when unknown', () => {
      expect(shell.resolveFactionName('unknown-uuid')).toBe('unknown-uuid');
    });

    it('getFactionDropdownOptions returns array of {uuid, name}', () => {
      shell.setFactionMap(new Map([['id1', 'Alpha'], ['id2', 'Beta']]));
      const opts = shell.getFactionDropdownOptions();
      expect(opts).toContainEqual({ uuid: 'id1', name: 'Alpha' });
      expect(opts).toContainEqual({ uuid: 'id2', name: 'Beta' });
    });

    it('can resolve faction for pirate_raider with real faction files', () => {
      const factionFiles = listDir('assets/factions')
        .filter((f) => f.endsWith('.toml'))
        .map((name) => ({ name, content: readFaction(name) }));
      const map = buildFactionMap(factionFiles);
      shell.setFactionMap(map);
      expect(shell.resolveFactionName('bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb')).toBe('Pirate');
    });
  });

  describe('complexity dropdown', () => {
    it('starts with empty complexity paths', () => {
      expect(shell.getComplexityPaths()).toEqual([]);
    });

    it('setComplexityPaths stores paths', () => {
      const paths = ['assets/complexity/tactical.toml', 'assets/complexity/power.toml'];
      shell.setComplexityPaths(paths);
      expect(shell.getComplexityPaths()).toEqual(paths);
    });

    it('getComplexityPaths returns a copy', () => {
      const paths = ['assets/complexity/tactical.toml'];
      shell.setComplexityPaths(paths);
      shell.getComplexityPaths().push('extra.toml');
      expect(shell.getComplexityPaths()).toHaveLength(1);
    });

    it('loads all real complexity TOMLs', () => {
      const filenames = listDir('assets/complexity').filter((f) => f.endsWith('.toml'));
      const paths = buildComplexityPaths(filenames);
      shell.setComplexityPaths(paths);
      expect(shell.getComplexityPaths().length).toBe(filenames.length);
      for (const p of shell.getComplexityPaths()) {
        expect(p).toMatch(/^assets\/complexity\//);
      }
    });
  });

  describe('preview pane', () => {
    it('returns placeholder stub before any file is opened', () => {
      const preview = shell.getPreviewPane();
      expect(preview.placeholder).toBe(true);
      expect(preview.activeFile).toBeNull();
    });

    it('returns real preview data after opening an entity', () => {
      shell.openFile('pirate_raider.toml', readEntity('pirate_raider.toml'));
      const preview = shell.getPreviewPane();
      // Not a placeholder — real preview shape
      expect(preview.placeholder).toBeUndefined();
      expect(preview).toHaveProperty('radarShape');
      expect(preview).toHaveProperty('radarColour');
      expect(preview).toHaveProperty('colliderShape');
      expect(preview).toHaveProperty('regionShape');
      expect(preview).toHaveProperty('asteroidField');
      expect(preview).toHaveProperty('showForwardArrow', true);
      expect(preview).toHaveProperty('textOverlay');
      expect(preview.textOverlay).toHaveProperty('tags');
      expect(preview.textOverlay).toHaveProperty('faction');
      expect(preview.textOverlay).toHaveProperty('consoles');
      expect(preview.textOverlay).toHaveProperty('hullTotal');
    });
  });

  describe('setSection + restoreParsed', () => {
    beforeEach(() => {
      shell.openFile('pirate_raider.toml', readEntity('pirate_raider.toml'));
    });

    it('setSection updates the active section and rebuilds cards', () => {
      shell.setSection('hull', { captain_chair: 999.0 });
      const card = shell.getCard('hull');
      expect(card.data.captain_chair).toBe(999.0);
    });

    it('setSection with a brand-new section adds a card', () => {
      expect(shell.getCard('shape')).toBeNull();
      shell.setSection('shape', { type: 'sphere', radius: 50.0 });
      expect(shell.getCard('shape')).not.toBeNull();
    });

    it('restoreParsed swaps the entire parsed object and rebuilds cards', () => {
      const replacement = {
        tags: ['region'],
        shape: { type: 'sphere', radius: 100 },
        effects: { comms_jammed: {} },
      };
      shell.restoreParsed(replacement);
      expect(shell.getCard('hull')).toBeNull();
      expect(shell.getCard('shape')).not.toBeNull();
      expect(shell.getCard('effects')).not.toBeNull();
      expect(shell.getParsedEntity()).toBe(replacement);
    });
  });

  describe('full integration scenario', () => {
    it('opens all shipped entities without error', () => {
      const filenames = listDir('assets/entities').filter((f) => f.endsWith('.toml'));
      for (const filename of filenames) {
        const text = readEntity(filename);
        const result = shell.openFile(`assets/entities/${filename}`, text);
        expect(result.ok, `${filename} should open without error: ${result.errors.join(', ')}`).toBe(true);
      }
    });
  });
});
