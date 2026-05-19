import { describe, it, expect, beforeEach } from 'vitest';
import { readFileSync, readdirSync } from 'fs';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';
import { parse } from 'smol-toml';

import {
  parseComplexityToml,
  stringifyComplexityToml,
  KNOWN_UI_ELEMENTS,
  ComplexityEditor,
} from '../complexity-editor.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '../..');
const complexityDir = resolve(projectRoot, 'assets/complexity');

function readComplexity(name) {
  return readFileSync(resolve(complexityDir, name), 'utf-8');
}

function allComplexityFiles() {
  return readdirSync(complexityDir)
    .filter((f) => f.endsWith('.toml'))
    .map((name) => ({
      path: `assets/complexity/${name}`,
      content: readComplexity(name),
    }));
}

// ── parseComplexityToml ───────────────────────────────────────────────────────

describe('parseComplexityToml', () => {
  it('parses a minimal single-preset file', () => {
    const toml = `[[preset]]\nname = "Std"\nhidden_elements = []\n`;
    const presets = parseComplexityToml(toml);
    expect(presets).toHaveLength(1);
    expect(presets[0].name).toBe('Std');
    expect(presets[0].hidden_elements).toEqual([]);
    expect(presets[0].delegated).toEqual({});
    expect(presets[0].ai).toEqual({});
  });

  it('parses hidden_elements array', () => {
    const toml = `[[preset]]\nname = "Low"\nhidden_elements = ["phaser_mode_selector", "torpedo_tube_selector"]\n`;
    const presets = parseComplexityToml(toml);
    expect(presets[0].hidden_elements).toEqual(['phaser_mode_selector', 'torpedo_tube_selector']);
  });

  it('parses delegated block', () => {
    const toml = `[[preset]]\nname = "Low"\n\n[preset.delegated]\nTactical = { controls = ["auto_fire_torpedoes"] }\n`;
    const presets = parseComplexityToml(toml);
    expect(presets[0].delegated).toEqual({
      Tactical: { controls: ['auto_fire_torpedoes'] },
    });
  });

  it('parses ai block with numeric and boolean params', () => {
    const toml = `[[preset]]\nname = "Low"\n\n[preset.ai]\ntorpedo_auto_fire = { lead_prediction = true, min_accuracy = 0.7 }\n`;
    const presets = parseComplexityToml(toml);
    expect(presets[0].ai.torpedo_auto_fire.lead_prediction).toBe(true);
    expect(presets[0].ai.torpedo_auto_fire.min_accuracy).toBeCloseTo(0.7);
  });

  it('parses two presets', () => {
    const toml = `[[preset]]\nname = "Low"\nhidden_elements = ["btn"]\n\n[[preset]]\nname = "Std"\nhidden_elements = []\n`;
    const presets = parseComplexityToml(toml);
    expect(presets).toHaveLength(2);
    expect(presets[0].name).toBe('Low');
    expect(presets[1].name).toBe('Std');
  });

  it('defaults delegated and ai to empty objects when absent', () => {
    const toml = `[[preset]]\nname = "Std"\n`;
    const presets = parseComplexityToml(toml);
    expect(presets[0].delegated).toEqual({});
    expect(presets[0].ai).toEqual({});
  });

  it('throws when preset array is missing', () => {
    const toml = `name = "oops"\n`;
    expect(() => parseComplexityToml(toml)).toThrow(/preset/);
  });

  it('throws on invalid TOML syntax', () => {
    expect(() => parseComplexityToml('[[preset\nname = bad')).toThrow();
  });
});

// ── stringifyComplexityToml ───────────────────────────────────────────────────

describe('stringifyComplexityToml', () => {
  it('serializes a minimal preset and produces valid TOML', () => {
    const presets = [{ name: 'Std', hidden_elements: [], delegated: {}, ai: {} }];
    const toml = stringifyComplexityToml(presets);
    expect(typeof toml).toBe('string');
    const reparsed = parse(toml);
    expect(Array.isArray(reparsed.preset)).toBe(true);
    expect(reparsed.preset[0].name).toBe('Std');
  });

  it('round-trips hidden_elements', () => {
    const presets = [{ name: 'Low', hidden_elements: ['phaser_mode_selector'], delegated: {}, ai: {} }];
    const toml = stringifyComplexityToml(presets);
    const reparsed = parseComplexityToml(toml);
    expect(reparsed[0].hidden_elements).toEqual(['phaser_mode_selector']);
  });

  it('round-trips delegated block', () => {
    const presets = [{
      name: 'Low',
      hidden_elements: [],
      delegated: { Tactical: { controls: ['auto_fire_torpedoes', 'auto_frequency_match'] } },
      ai: {},
    }];
    const toml = stringifyComplexityToml(presets);
    const reparsed = parseComplexityToml(toml);
    expect(reparsed[0].delegated.Tactical.controls).toEqual(['auto_fire_torpedoes', 'auto_frequency_match']);
  });

  it('round-trips ai block', () => {
    const presets = [{
      name: 'Low',
      hidden_elements: [],
      delegated: {},
      ai: { torpedo_auto_fire: { lead_prediction: true, min_accuracy: 0.7 } },
    }];
    const toml = stringifyComplexityToml(presets);
    const reparsed = parseComplexityToml(toml);
    expect(reparsed[0].ai.torpedo_auto_fire.lead_prediction).toBe(true);
    expect(reparsed[0].ai.torpedo_auto_fire.min_accuracy).toBeCloseTo(0.7);
  });
});

// ── KNOWN_UI_ELEMENTS ─────────────────────────────────────────────────────────

describe('KNOWN_UI_ELEMENTS', () => {
  it('has an entry for tactical', () => {
    expect(Array.isArray(KNOWN_UI_ELEMENTS.tactical)).toBe(true);
    expect(KNOWN_UI_ELEMENTS.tactical.length).toBeGreaterThan(0);
  });

  it('tactical entry contains phaser_mode_selector', () => {
    expect(KNOWN_UI_ELEMENTS.tactical).toContain('phaser_mode_selector');
  });

  it('has entries for all shipped consoles', () => {
    const consoles = ['tactical', 'science', 'power', 'helm', 'sensors', 'shields', 'navigation'];
    for (const c of consoles) {
      expect(Array.isArray(KNOWN_UI_ELEMENTS[c]), `missing: ${c}`).toBe(true);
    }
  });
});

// ── ComplexityEditor — file list ──────────────────────────────────────────────

describe('ComplexityEditor — file list', () => {
  let editor;

  beforeEach(() => {
    editor = new ComplexityEditor();
    editor.loadAll(allComplexityFiles());
  });

  it('shows every assets/complexity/*.toml file', () => {
    const list = editor.getFileList();
    expect(list.length).toBeGreaterThanOrEqual(6);
    for (const entry of list) {
      expect(entry).toMatch(/^assets\/complexity\/.+\.toml$/);
    }
  });

  it('includes tactical, power, science, shields, sensors, navigation', () => {
    const list = editor.getFileList();
    expect(list).toContain('assets/complexity/tactical.toml');
    expect(list).toContain('assets/complexity/power.toml');
    expect(list).toContain('assets/complexity/science.toml');
    expect(list).toContain('assets/complexity/shields.toml');
    expect(list).toContain('assets/complexity/sensors.toml');
    expect(list).toContain('assets/complexity/navigation.toml');
  });

  it('starts with no active file', () => {
    expect(editor.getActiveFile()).toBeNull();
  });

  it('starts with no presets', () => {
    expect(editor.getPresets()).toBeNull();
  });
});

// ── ComplexityEditor — openFile ───────────────────────────────────────────────

describe('ComplexityEditor — openFile', () => {
  let editor;

  beforeEach(() => {
    editor = new ComplexityEditor();
    editor.loadAll(allComplexityFiles());
  });

  it('opens a file and sets activeFile', () => {
    const ok = editor.openFile('assets/complexity/tactical.toml');
    expect(ok).toBe(true);
    expect(editor.getActiveFile()).toBe('assets/complexity/tactical.toml');
  });

  it('returns false for a nonexistent file', () => {
    expect(editor.openFile('assets/complexity/nonexistent.toml')).toBe(false);
  });

  it('tactical file has two presets (Low, Std)', () => {
    editor.openFile('assets/complexity/tactical.toml');
    const presets = editor.getPresets();
    expect(presets).toHaveLength(2);
    expect(presets[0].name).toBe('Low');
    expect(presets[1].name).toBe('Std');
  });

  it('tactical Low preset has hidden_elements', () => {
    editor.openFile('assets/complexity/tactical.toml');
    const low = editor.getPreset(0);
    expect(low.hidden_elements).toContain('phaser_mode_selector');
    expect(low.hidden_elements).toContain('torpedo_tube_selector');
  });

  it('tactical Low preset has delegated block', () => {
    editor.openFile('assets/complexity/tactical.toml');
    const low = editor.getPreset(0);
    expect(low.delegated).toHaveProperty('Tactical');
    expect(low.delegated.Tactical.controls).toContain('auto_fire_torpedoes');
  });

  it('tactical Low preset has ai block', () => {
    editor.openFile('assets/complexity/tactical.toml');
    const low = editor.getPreset(0);
    expect(low.ai).toHaveProperty('torpedo_auto_fire');
    expect(low.ai.torpedo_auto_fire.min_accuracy).toBeCloseTo(0.7);
  });

  it('getPresets returns deep copies (mutations do not affect internal state)', () => {
    editor.openFile('assets/complexity/tactical.toml');
    const presets = editor.getPresets();
    presets[0].name = 'MUTATED';
    expect(editor.getPreset(0).name).toBe('Low');
  });

  it('getPreset returns null for out-of-range index', () => {
    editor.openFile('assets/complexity/tactical.toml');
    expect(editor.getPreset(99)).toBeNull();
  });
});

// ── ComplexityEditor — getKnownUiElements ────────────────────────────────────

describe('ComplexityEditor — getKnownUiElements', () => {
  let editor;

  beforeEach(() => {
    editor = new ComplexityEditor();
    editor.loadAll(allComplexityFiles());
  });

  it('returns empty array when no file is open', () => {
    expect(editor.getKnownUiElements()).toEqual([]);
  });

  it('returns tactical UI elements when tactical file is open', () => {
    editor.openFile('assets/complexity/tactical.toml');
    const elements = editor.getKnownUiElements();
    expect(Array.isArray(elements)).toBe(true);
    expect(elements).toContain('phaser_mode_selector');
  });

  it('returns power UI elements when power file is open', () => {
    editor.openFile('assets/complexity/power.toml');
    const elements = editor.getKnownUiElements();
    expect(elements).toContain('power_overflow_controls');
  });
});

// ── ComplexityEditor — setHiddenElements ─────────────────────────────────────

describe('ComplexityEditor — setHiddenElements', () => {
  let editor;

  beforeEach(() => {
    editor = new ComplexityEditor();
    editor.loadAll(allComplexityFiles());
    editor.openFile('assets/complexity/tactical.toml');
  });

  it('replaces hidden_elements for a preset', () => {
    editor.setHiddenElements(0, ['phaser_mode_selector']);
    expect(editor.getPreset(0).hidden_elements).toEqual(['phaser_mode_selector']);
  });

  it('can set hidden_elements to empty', () => {
    editor.setHiddenElements(0, []);
    expect(editor.getPreset(0).hidden_elements).toEqual([]);
  });

  it('is a no-op when no file is open', () => {
    const fresh = new ComplexityEditor();
    fresh.loadAll(allComplexityFiles());
    expect(() => fresh.setHiddenElements(0, ['btn'])).not.toThrow();
    expect(fresh.getPresets()).toBeNull();
  });

  it('is a no-op for out-of-range index', () => {
    const before = editor.getPreset(0).hidden_elements;
    editor.setHiddenElements(99, ['btn']);
    expect(editor.getPreset(0).hidden_elements).toEqual(before);
  });
});

// ── ComplexityEditor — setDelegated / removeDelegated ────────────────────────

describe('ComplexityEditor — delegated block', () => {
  let editor;

  beforeEach(() => {
    editor = new ComplexityEditor();
    editor.loadAll(allComplexityFiles());
    editor.openFile('assets/complexity/tactical.toml');
  });

  it('setDelegated updates controls for a console key', () => {
    editor.setDelegated(0, 'Tactical', ['auto_fire_torpedoes', 'auto_frequency_match']);
    expect(editor.getPreset(0).delegated.Tactical.controls).toEqual([
      'auto_fire_torpedoes',
      'auto_frequency_match',
    ]);
  });

  it('setDelegated creates a new entry if key did not exist', () => {
    editor.setDelegated(0, 'Helm', ['auto_steering']);
    expect(editor.getPreset(0).delegated.Helm.controls).toEqual(['auto_steering']);
  });

  it('removeDelegated removes the console key', () => {
    editor.removeDelegated(0, 'Tactical');
    expect(editor.getPreset(0).delegated).not.toHaveProperty('Tactical');
  });

  it('removeDelegated is a no-op for a non-existent key', () => {
    expect(() => editor.removeDelegated(0, 'NonExistent')).not.toThrow();
  });
});

// ── ComplexityEditor — AI tuning ──────────────────────────────────────────────

describe('ComplexityEditor — AI tuning', () => {
  let editor;

  beforeEach(() => {
    editor = new ComplexityEditor();
    editor.loadAll(allComplexityFiles());
    editor.openFile('assets/complexity/tactical.toml');
  });

  it('setAiParam updates a single param in an existing behavior block', () => {
    editor.setAiParam(0, 'torpedo_auto_fire', 'min_accuracy', 0.9);
    expect(editor.getPreset(0).ai.torpedo_auto_fire.min_accuracy).toBeCloseTo(0.9);
  });

  it('setAiParam creates a new behavior block if it does not exist', () => {
    editor.setAiParam(0, 'new_behavior', 'threshold', 0.5);
    expect(editor.getPreset(0).ai.new_behavior.threshold).toBeCloseTo(0.5);
  });

  it('setAiBlock replaces all params for a behavior block', () => {
    editor.setAiBlock(0, 'torpedo_auto_fire', { min_accuracy: 0.95, lead_prediction: false });
    const block = editor.getPreset(0).ai.torpedo_auto_fire;
    expect(block.min_accuracy).toBeCloseTo(0.95);
    expect(block.lead_prediction).toBe(false);
    // old params from the original block are gone
    expect(Object.keys(block)).toEqual(['min_accuracy', 'lead_prediction']);
  });

  it('removeAiBlock removes the behavior entry', () => {
    editor.removeAiBlock(0, 'torpedo_auto_fire');
    expect(editor.getPreset(0).ai).not.toHaveProperty('torpedo_auto_fire');
  });

  it('setAiParam is a no-op when no file is open', () => {
    const fresh = new ComplexityEditor();
    fresh.loadAll(allComplexityFiles());
    expect(() => fresh.setAiParam(0, 'x', 'y', 1)).not.toThrow();
    expect(fresh.getPresets()).toBeNull();
  });
});

// ── ComplexityEditor — serialize ──────────────────────────────────────────────

describe('ComplexityEditor — serialize', () => {
  let editor;

  beforeEach(() => {
    editor = new ComplexityEditor();
    editor.loadAll(allComplexityFiles());
  });

  it('throws when no file is open', () => {
    expect(() => editor.serialize()).toThrow(/No complexity file/);
  });

  it('serializes the current presets to a valid TOML string', () => {
    editor.openFile('assets/complexity/tactical.toml');
    const toml = editor.serialize();
    expect(typeof toml).toBe('string');
    const reparsed = parse(toml);
    expect(Array.isArray(reparsed.preset)).toBe(true);
  });

  it('preserves preset names after serialization', () => {
    editor.openFile('assets/complexity/tactical.toml');
    const toml = editor.serialize();
    const reparsed = parseComplexityToml(toml);
    expect(reparsed[0].name).toBe('Low');
    expect(reparsed[1].name).toBe('Std');
  });
});

// ── Round-trip: load fixture → modify → serialize ────────────────────────────

describe('Round-trip: load → modify → serialize → deep-equal expected', () => {
  it('round-trips tactical.toml: modify hidden_elements, delegated, ai', () => {
    const editor = new ComplexityEditor();
    editor.loadAll(allComplexityFiles());
    editor.openFile('assets/complexity/tactical.toml');

    // Initial check
    expect(editor.getPreset(0).name).toBe('Low');

    // Modify hidden elements for Low preset
    editor.setHiddenElements(0, ['phaser_mode_selector']);

    // Modify delegated
    editor.setDelegated(0, 'Tactical', ['auto_fire_torpedoes']);

    // Modify AI
    editor.setAiParam(0, 'torpedo_auto_fire', 'min_accuracy', 0.85);

    const toml = editor.serialize();
    const reparsed = parseComplexityToml(toml);

    expect(reparsed[0].hidden_elements).toEqual(['phaser_mode_selector']);
    expect(reparsed[0].delegated.Tactical.controls).toEqual(['auto_fire_torpedoes']);
    expect(reparsed[0].ai.torpedo_auto_fire.min_accuracy).toBeCloseTo(0.85);
    // Std preset preserved
    expect(reparsed[1].name).toBe('Std');
    expect(reparsed[1].hidden_elements).toEqual([]);
  });

  it('round-trips all shipped complexity files unchanged', () => {
    const files = allComplexityFiles();
    const editor = new ComplexityEditor();
    editor.loadAll(files);

    for (const { path, content } of files) {
      editor.openFile(path);
      const originalPresets = parseComplexityToml(content);
      const serialized = editor.serialize();
      const reparsed = parseComplexityToml(serialized);

      expect(reparsed.length).toBe(originalPresets.length);
      for (let i = 0; i < originalPresets.length; i++) {
        expect(reparsed[i].name).toBe(originalPresets[i].name);
        expect(reparsed[i].hidden_elements).toEqual(originalPresets[i].hidden_elements);
        // delegated keys must round-trip
        for (const [key, val] of Object.entries(originalPresets[i].delegated)) {
          expect(reparsed[i].delegated[key]?.controls).toEqual(val.controls);
        }
        // ai behavior keys must round-trip
        for (const [key, params] of Object.entries(originalPresets[i].ai)) {
          for (const [pk, pv] of Object.entries(params)) {
            expect(reparsed[i].ai[key]?.[pk]).toBeCloseTo(
              typeof pv === 'number' ? pv : pv,
              5,
            );
          }
        }
      }
    }
  });

  it('power.toml: modify and verify round-trip', () => {
    const editor = new ComplexityEditor();
    editor.loadAll(allComplexityFiles());
    editor.openFile('assets/complexity/power.toml');

    // power has Low and Std
    const presets = editor.getPresets();
    expect(presets[0].name).toBe('Low');

    // Add a new hidden element
    editor.setHiddenElements(0, ['power_overflow_controls', 'battery_level_readout']);

    const toml = editor.serialize();
    const reparsed = parseComplexityToml(toml);
    expect(reparsed[0].hidden_elements).toContain('battery_level_readout');
  });
});
