import { describe, it, expect, beforeEach } from 'vitest';
import { readFileSync, readdirSync } from 'fs';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';
import { parse } from 'smol-toml';

import {
  parseFactionToml,
  stringifyFactionToml,
  FactionEditor,
} from '../faction-editor.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '../..');
const factionsDir = resolve(projectRoot, 'assets/factions');

function readFaction(name) {
  return readFileSync(resolve(factionsDir, name), 'utf-8');
}

function allFactionFiles() {
  return readdirSync(factionsDir)
    .filter((f) => f.endsWith('.toml'))
    .map((name) => ({
      path: `assets/factions/${name}`,
      content: readFaction(name),
    }));
}

// ── parseFactionToml ──────────────────────────────────────────────────────────

describe('parseFactionToml', () => {
  it('parses a valid faction TOML', () => {
    const toml = `uuid = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"\nname = "Federation"\nenemies = ["bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb"]\n`;
    const result = parseFactionToml(toml);
    expect(result.uuid).toBe('aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa');
    expect(result.name).toBe('Federation');
    expect(result.enemies).toEqual(['bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb']);
  });

  it('parses a faction with empty enemies', () => {
    const toml = `uuid = "dddddddd-4444-4444-8444-dddddddddddd"\nname = "Requiem"\nenemies = []\n`;
    const result = parseFactionToml(toml);
    expect(result.enemies).toEqual([]);
  });

  it('throws if uuid is missing', () => {
    const toml = `name = "X"\nenemies = []\n`;
    expect(() => parseFactionToml(toml)).toThrow(/uuid/);
  });

  it('throws if name is missing', () => {
    const toml = `uuid = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"\nenemies = []\n`;
    expect(() => parseFactionToml(toml)).toThrow(/name/);
  });

  it('throws if enemies is missing', () => {
    const toml = `uuid = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"\nname = "X"\n`;
    expect(() => parseFactionToml(toml)).toThrow(/enemies/);
  });

  it('throws on invalid TOML syntax', () => {
    expect(() => parseFactionToml('not : valid : toml =')).toThrow();
  });
});

// ── stringifyFactionToml ──────────────────────────────────────────────────────

describe('stringifyFactionToml', () => {
  it('serializes a faction to TOML', () => {
    const faction = {
      uuid: 'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa',
      name: 'Federation',
      enemies: ['bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb'],
    };
    const toml = stringifyFactionToml(faction);
    expect(typeof toml).toBe('string');
    expect(toml).toContain('uuid');
    expect(toml).toContain('name');
    expect(toml).toContain('enemies');
  });

  it('produces parseable TOML', () => {
    const faction = {
      uuid: 'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa',
      name: 'Federation',
      enemies: ['bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb'],
    };
    const toml = stringifyFactionToml(faction);
    const reparsed = parse(toml);
    expect(reparsed.uuid).toBe(faction.uuid);
    expect(reparsed.name).toBe(faction.name);
    expect(reparsed.enemies).toEqual(faction.enemies);
  });

  it('writes enemies as a UUID string array', () => {
    const faction = {
      uuid: 'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa',
      name: 'Test',
      enemies: ['bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb'],
    };
    const toml = stringifyFactionToml(faction);
    const reparsed = parse(toml);
    expect(Array.isArray(reparsed.enemies)).toBe(true);
    expect(reparsed.enemies[0]).toBe('bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb');
  });
});

// ── FactionEditor — file list ─────────────────────────────────────────────────

describe('FactionEditor — file list', () => {
  let editor;

  beforeEach(() => {
    editor = new FactionEditor();
    editor.loadAll(allFactionFiles());
  });

  it('shows every assets/factions/*.toml file', () => {
    const list = editor.getFileList();
    expect(list.length).toBeGreaterThanOrEqual(4);
    for (const entry of list) {
      expect(entry).toMatch(/^assets\/factions\/.+\.toml$/);
    }
  });

  it('includes federation, harrow, pirate, requiem', () => {
    const list = editor.getFileList();
    expect(list).toContain('assets/factions/federation.toml');
    expect(list).toContain('assets/factions/harrow.toml');
    expect(list).toContain('assets/factions/pirate.toml');
    expect(list).toContain('assets/factions/requiem.toml');
  });

  it('starts with no active file', () => {
    expect(editor.getActiveFile()).toBeNull();
  });
});

// ── FactionEditor — openFile / form ──────────────────────────────────────────

describe('FactionEditor — openFile and form state', () => {
  let editor;

  beforeEach(() => {
    editor = new FactionEditor();
    editor.loadAll(allFactionFiles());
  });

  it('opens a faction file and populates form state', () => {
    const opened = editor.openFile('assets/factions/federation.toml');
    expect(opened).toBe(true);
    expect(editor.getActiveFile()).toBe('assets/factions/federation.toml');
    const state = editor.getFormState();
    expect(state.uuid).toBe('aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa');
    expect(state.name).toBe('Federation');
    expect(Array.isArray(state.enemies)).toBe(true);
  });

  it('returns false for a nonexistent file', () => {
    expect(editor.openFile('assets/factions/nonexistent.toml')).toBe(false);
  });

  it('uuid is read-only (form state cannot change it)', () => {
    editor.openFile('assets/factions/federation.toml');
    const state = editor.getFormState();
    const originalUuid = state.uuid;
    // Mutating returned state should not change internal state
    state.uuid = 'CHANGED';
    expect(editor.getFormState().uuid).toBe(originalUuid);
  });
});

// ── FactionEditor — setName ───────────────────────────────────────────────────

describe('FactionEditor — setName', () => {
  let editor;

  beforeEach(() => {
    editor = new FactionEditor();
    editor.loadAll(allFactionFiles());
    editor.openFile('assets/factions/federation.toml');
  });

  it('updates the name in the form state', () => {
    editor.setName('New Federation');
    expect(editor.getFormState().name).toBe('New Federation');
  });

  it('does not change uuid when name is updated', () => {
    const originalUuid = editor.getFormState().uuid;
    editor.setName('Changed Name');
    expect(editor.getFormState().uuid).toBe(originalUuid);
  });

  it('is a no-op when no file is open', () => {
    const fresh = new FactionEditor();
    fresh.loadAll(allFactionFiles());
    expect(() => fresh.setName('X')).not.toThrow();
    expect(fresh.getFormState()).toBeNull();
  });
});

// ── FactionEditor — enemy multi-select ───────────────────────────────────────

describe('FactionEditor — getEnemyOptions', () => {
  let editor;

  beforeEach(() => {
    editor = new FactionEditor();
    editor.loadAll(allFactionFiles());
    editor.openFile('assets/factions/federation.toml');
  });

  it('returns all factions except the open one', () => {
    const options = editor.getEnemyOptions();
    const paths = options.map((o) => o.path);
    expect(paths).not.toContain('assets/factions/federation.toml');
    expect(paths).toContain('assets/factions/harrow.toml');
    expect(paths).toContain('assets/factions/pirate.toml');
    expect(paths).toContain('assets/factions/requiem.toml');
  });

  it('each option has uuid, name, path fields', () => {
    const options = editor.getEnemyOptions();
    for (const opt of options) {
      expect(typeof opt.uuid).toBe('string');
      expect(typeof opt.name).toBe('string');
      expect(typeof opt.path).toBe('string');
    }
  });

  it('options are identified by name', () => {
    const options = editor.getEnemyOptions();
    const names = options.map((o) => o.name);
    expect(names).toContain('Harrow');
    expect(names).toContain('Pirate');
    expect(names).toContain('Requiem');
  });
});

// ── FactionEditor — setEnemies ────────────────────────────────────────────────

describe('FactionEditor — setEnemies', () => {
  let editor;

  beforeEach(() => {
    editor = new FactionEditor();
    editor.loadAll(allFactionFiles());
    editor.openFile('assets/factions/federation.toml');
  });

  it('replaces enemies list in form state', () => {
    const newEnemies = ['cccccccc-3333-4333-8333-cccccccccccc'];
    editor.setEnemies(newEnemies);
    expect(editor.getFormState().enemies).toEqual(newEnemies);
  });

  it('can set enemies to empty', () => {
    editor.setEnemies([]);
    expect(editor.getFormState().enemies).toEqual([]);
  });

  it('is a no-op when no file is open', () => {
    const fresh = new FactionEditor();
    fresh.loadAll(allFactionFiles());
    expect(() => fresh.setEnemies(['some-uuid'])).not.toThrow();
    expect(fresh.getFormState()).toBeNull();
  });
});

// ── FactionEditor — serialize (save) ─────────────────────────────────────────

describe('FactionEditor — serialize', () => {
  let editor;

  beforeEach(() => {
    editor = new FactionEditor();
    editor.loadAll(allFactionFiles());
  });

  it('throws when no file is open', () => {
    expect(() => editor.serialize()).toThrow(/No faction file/);
  });

  it('serializes the form state to TOML', () => {
    editor.openFile('assets/factions/federation.toml');
    const toml = editor.serialize();
    expect(typeof toml).toBe('string');
    const reparsed = parse(toml);
    expect(reparsed.uuid).toBe('aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa');
    expect(reparsed.name).toBe('Federation');
  });

  it('writes enemies as a UUID array', () => {
    editor.openFile('assets/factions/requiem.toml');
    editor.setEnemies(['aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa']);
    const toml = editor.serialize();
    const reparsed = parse(toml);
    expect(Array.isArray(reparsed.enemies)).toBe(true);
    expect(reparsed.enemies).toEqual(['aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa']);
  });
});

// ── Round-trip: load fixture → modify name → modify enemies → serialize ───────

describe('Round-trip: load → modify → serialize → deep-equal expected', () => {
  it('matches expected TOML after name and enemies changes', () => {
    const editor = new FactionEditor();
    editor.loadAll(allFactionFiles());

    // Open the Requiem faction (starts with empty enemies)
    editor.openFile('assets/factions/requiem.toml');

    // Confirm initial state
    const initialState = editor.getFormState();
    expect(initialState.uuid).toBe('dddddddd-4444-4444-8444-dddddddddddd');
    expect(initialState.name).toBe('Requiem');
    expect(initialState.enemies).toEqual([]);

    // Modify name
    editor.setName('Requiem Reborn');

    // Modify enemies — add Federation and Pirate by UUID
    editor.setEnemies([
      'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa',
      'bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb',
    ]);

    // Serialize
    const toml = editor.serialize();
    const reparsed = parse(toml);

    // Deep-equal expected state
    expect(reparsed).toEqual({
      uuid: 'dddddddd-4444-4444-8444-dddddddddddd',
      name: 'Requiem Reborn',
      enemies: [
        'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa',
        'bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb',
      ],
    });
  });

  it('round-trips all shipped faction files unchanged', () => {
    const files = allFactionFiles();
    const editor = new FactionEditor();
    editor.loadAll(files);

    for (const { path } of files) {
      editor.openFile(path);
      const stateBefore = editor.getFormState();
      const toml = editor.serialize();
      const reparsed = parse(toml);
      expect(reparsed.uuid).toBe(stateBefore.uuid);
      expect(reparsed.name).toBe(stateBefore.name);
      expect(reparsed.enemies).toEqual(stateBefore.enemies);
    }
  });
});
