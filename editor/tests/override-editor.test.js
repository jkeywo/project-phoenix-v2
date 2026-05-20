import { describe, it, expect, beforeEach } from 'vitest';
import { parse } from 'smol-toml';
import { readFileSync } from 'fs';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';

import { OverrideEditor, deepMerge } from '../override-editor.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '../..');

// ── Helpers ───────────────────────────────────────────────────────────────────

function readEntityToml(relPath) {
  return parse(readFileSync(resolve(projectRoot, relPath), 'utf-8'));
}

const PLAYER_SHIP = readEntityToml('assets/entities/player_ship.toml');

// ── deepMerge (unit) ──────────────────────────────────────────────────────────

describe('deepMerge', () => {
  it('returns override value for primitive clash', () => {
    expect(deepMerge({ a: 1 }, { a: 2 })).toEqual({ a: 2 });
  });

  it('preserves template keys absent from override', () => {
    const r = deepMerge({ a: 1, b: 2 }, { a: 99 });
    expect(r.a).toBe(99);
    expect(r.b).toBe(2);
  });

  it('adds keys absent from template', () => {
    const r = deepMerge({ a: 1 }, { b: 2 });
    expect(r.a).toBe(1);
    expect(r.b).toBe(2);
  });

  it('recursively merges nested objects', () => {
    const r = deepMerge(
      { hull: { max: 100, repair: 2 } },
      { hull: { max: 150 } },
    );
    expect(r.hull.max).toBe(150);
    expect(r.hull.repair).toBe(2);
  });

  it('replaces arrays wholesale (not element-wise)', () => {
    const r = deepMerge({ tags: ['a', 'b'] }, { tags: ['x'] });
    expect(r.tags).toEqual(['x']);
  });

  it('does not mutate template', () => {
    const template = { a: { b: 1 } };
    deepMerge(template, { a: { b: 2 } });
    expect(template.a.b).toBe(1);
  });

  it('does not mutate override', () => {
    const over = { a: 99 };
    deepMerge({ a: 1 }, over);
    expect(over.a).toBe(99);
  });

  it('handles override of false (falsy value should not be ignored)', () => {
    const r = deepMerge({ online: true }, { online: false });
    expect(r.online).toBe(false);
  });
});

// ── OverrideEditor — construction ─────────────────────────────────────────────

describe('OverrideEditor — constructor', () => {
  it('resolved view equals template when no overrides applied', () => {
    const ed = new OverrideEditor({ hull: { max: 100 } });
    expect(ed.getResolvedView()).toEqual({ hull: { max: 100 } });
  });

  it('constructor does not retain reference to the original template', () => {
    const tmpl = { hull: { max: 100 } };
    const ed = new OverrideEditor(tmpl);
    tmpl.hull.max = 999;
    expect(ed.getResolvedView().hull.max).toBe(100);
  });

  it('starts with no overrides (empty summary)', () => {
    const ed = new OverrideEditor({ a: 1 });
    expect(ed.getOverridesSummary()).toEqual([]);
  });
});

// ── OverrideEditor — setOverride ──────────────────────────────────────────────

describe('OverrideEditor — setOverride', () => {
  let ed;

  beforeEach(() => {
    ed = new OverrideEditor(PLAYER_SHIP);
  });

  it('overrides a top-level field', () => {
    ed.setOverride('faction', 'bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb');
    expect(ed.getResolvedView().faction).toBe('bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb');
  });

  it('overrides a nested field without disturbing siblings', () => {
    const originalRadius = PLAYER_SHIP.radar_appearance.radius;
    ed.setOverride('radar_appearance.colour', [1.0, 0.0, 0.0]);
    const resolved = ed.getResolvedView();
    expect(resolved.radar_appearance.colour).toEqual([1.0, 0.0, 0.0]);
    expect(resolved.radar_appearance.radius).toBe(originalRadius);
  });

  it('overrides a deeply nested field (three levels)', () => {
    ed.setOverride('helm_console.max_speed', 80.0);
    expect(ed.getResolvedView().helm_console.max_speed).toBe(80.0);
    // siblings in helm_console preserved
    expect(ed.getResolvedView().helm_console.acceleration).toBe(PLAYER_SHIP.helm_console.acceleration);
  });

  it('overrides an array field wholesale', () => {
    ed.setOverride('tags', ['enemy', 'ship']);
    expect(ed.getResolvedView().tags).toEqual(['enemy', 'ship']);
  });

  it('supports subfields absent from the template', () => {
    ed.setOverride('custom_module.power_draw', 42);
    const resolved = ed.getResolvedView();
    expect(resolved.custom_module.power_draw).toBe(42);
    // template unchanged
    expect(PLAYER_SHIP.custom_module).toBeUndefined();
  });

  it('supports subfields absent from the template — deeply nested', () => {
    ed.setOverride('extra.deep.value', 'hello');
    expect(ed.getResolvedView().extra.deep.value).toBe('hello');
  });

  it('successive calls accumulate overrides', () => {
    ed.setOverride('hull.repair_team_count', 4);
    ed.setOverride('radar_appearance.radius', 10.0);
    const resolved = ed.getResolvedView();
    expect(resolved.hull.repair_team_count).toBe(4);
    expect(resolved.radar_appearance.radius).toBe(10.0);
  });

  it('overwriting the same path replaces the previous override', () => {
    ed.setOverride('helm_console.max_speed', 80.0);
    ed.setOverride('helm_console.max_speed', 60.0);
    expect(ed.getResolvedView().helm_console.max_speed).toBe(60.0);
  });
});

// ── OverrideEditor — clearOverride ────────────────────────────────────────────

describe('OverrideEditor — clearOverride', () => {
  let ed;

  beforeEach(() => {
    ed = new OverrideEditor(PLAYER_SHIP);
  });

  it('removes an overridden field so resolved falls back to template', () => {
    const templateSpeed = PLAYER_SHIP.helm_console.max_speed;
    ed.setOverride('helm_console.max_speed', 99.0);
    ed.clearOverride('helm_console.max_speed');
    expect(ed.getResolvedView().helm_console.max_speed).toBe(templateSpeed);
  });

  it('removing one field does not affect other overrides', () => {
    ed.setOverride('helm_console.max_speed', 99.0);
    ed.setOverride('radar_appearance.radius', 15.0);
    ed.clearOverride('helm_console.max_speed');
    expect(ed.getResolvedView().radar_appearance.radius).toBe(15.0);
  });

  it('is a no-op when path was not overridden', () => {
    expect(() => ed.clearOverride('hull.repair_team_count')).not.toThrow();
    expect(ed.getOverridesSummary()).toEqual([]);
  });

  it('removing an override removes it from the summary', () => {
    ed.setOverride('hull.repair_team_count', 5);
    ed.clearOverride('hull.repair_team_count');
    const summary = ed.getOverridesSummary();
    expect(summary.find((e) => e.path === 'hull.repair_team_count')).toBeUndefined();
  });

  it('clears a subfield absent from template', () => {
    ed.setOverride('custom_module.power_draw', 42);
    ed.clearOverride('custom_module.power_draw');
    expect(ed.getResolvedView().custom_module).toBeUndefined();
  });
});

// ── OverrideEditor — getResolvedView ─────────────────────────────────────────

describe('OverrideEditor — getResolvedView', () => {
  it('equals template when no overrides set', () => {
    const tmpl = { a: 1, b: { c: 2 } };
    const ed = new OverrideEditor(tmpl);
    expect(ed.getResolvedView()).toEqual(tmpl);
  });

  it('returns a new object each call (mutations do not bleed back)', () => {
    const ed = new OverrideEditor({ a: 1 });
    const v1 = ed.getResolvedView();
    v1.a = 999;
    expect(ed.getResolvedView().a).toBe(1);
  });

  it('returns deep-merged view for multi-level template', () => {
    const template = {
      hull: { max: 100, repair: 2 },
      weapons: { damage: 10 },
    };
    const ed = new OverrideEditor(template);
    ed.setOverride('hull.max', 200);
    const r = ed.getResolvedView();
    expect(r.hull.max).toBe(200);
    expect(r.hull.repair).toBe(2);
    expect(r.weapons.damage).toBe(10);
  });

  it('player_ship.toml: resolved has all template sections', () => {
    const ed = new OverrideEditor(PLAYER_SHIP);
    ed.setOverride('radar_appearance.colour', [1.0, 0.0, 0.0]);
    const resolved = ed.getResolvedView();
    // All top-level sections from template should be present
    for (const key of Object.keys(PLAYER_SHIP)) {
      expect(key in resolved).toBe(true);
    }
  });
});

// ── OverrideEditor — getOverridesSummary ─────────────────────────────────────

describe('OverrideEditor — getOverridesSummary', () => {
  it('returns empty array when no overrides set', () => {
    const ed = new OverrideEditor({ a: 1 });
    expect(ed.getOverridesSummary()).toEqual([]);
  });

  it('returns one entry per leaf path', () => {
    const ed = new OverrideEditor({ a: { b: 1 }, c: 2 });
    ed.setOverride('a.b', 99);
    ed.setOverride('c', 42);
    const summary = ed.getOverridesSummary();
    expect(summary).toHaveLength(2);
    const paths = summary.map((e) => e.path);
    expect(paths).toContain('a.b');
    expect(paths).toContain('c');
  });

  it('includes the correct values for each path', () => {
    const ed = new OverrideEditor({});
    ed.setOverride('radar_appearance.colour', [1.0, 0.0, 0.0]);
    ed.setOverride('hull.max', 200);
    const summary = ed.getOverridesSummary();
    const colourEntry = summary.find((e) => e.path === 'radar_appearance.colour');
    const hullEntry = summary.find((e) => e.path === 'hull.max');
    expect(colourEntry?.value).toEqual([1.0, 0.0, 0.0]);
    expect(hullEntry?.value).toBe(200);
  });

  it('array value appears as a single entry (not element-wise)', () => {
    const ed = new OverrideEditor({});
    ed.setOverride('tags', ['enemy', 'ship']);
    const summary = ed.getOverridesSummary();
    expect(summary).toHaveLength(1);
    expect(summary[0].value).toEqual(['enemy', 'ship']);
  });
});

// ── OverrideEditor — toOverridesToml ─────────────────────────────────────────

describe('OverrideEditor — toOverridesToml', () => {
  it('returns empty string when no overrides set', () => {
    const ed = new OverrideEditor({ a: 1 });
    expect(ed.toOverridesToml()).toBe('');
  });

  it('produces valid TOML that round-trips', () => {
    const ed = new OverrideEditor(PLAYER_SHIP);
    ed.setOverride('helm_console.max_speed', 80.0);
    ed.setOverride('radar_appearance.colour', [0.0, 1.0, 0.0]);
    const toml = ed.toOverridesToml();
    expect(typeof toml).toBe('string');
    const reparsed = parse(toml);
    expect(reparsed.helm_console.max_speed).toBe(80.0);
    expect(reparsed.radar_appearance.colour).toEqual([0.0, 1.0, 0.0]);
  });

  it('TOML contains only overridden fields, not the whole template', () => {
    const ed = new OverrideEditor(PLAYER_SHIP);
    ed.setOverride('radar_appearance.colour', [0.0, 1.0, 0.0]);
    const reparsed = parse(ed.toOverridesToml());
    // Only radar_appearance should appear (one key)
    expect(Object.keys(reparsed)).toHaveLength(1);
    expect(reparsed.radar_appearance).toBeDefined();
  });

  it('TOML for a top-level scalar override round-trips', () => {
    const ed = new OverrideEditor(PLAYER_SHIP);
    ed.setOverride('faction', 'ffffffff-ffff-4fff-8fff-ffffffffffff');
    const reparsed = parse(ed.toOverridesToml());
    expect(reparsed.faction).toBe('ffffffff-ffff-4fff-8fff-ffffffffffff');
  });

  it('TOML for a novel subfield absent from template is correct', () => {
    const ed = new OverrideEditor(PLAYER_SHIP);
    ed.setOverride('custom_module.power_draw', 42);
    const reparsed = parse(ed.toOverridesToml());
    expect(reparsed.custom_module.power_draw).toBe(42);
  });
});

// ── Integration: apply + clear + resolved + TOML cycle ───────────────────────

describe('Integration: apply → clear → resolve → serialise', () => {
  it('full round-trip: set, clear one, serialise, re-parse, verify', () => {
    const ed = new OverrideEditor(PLAYER_SHIP);

    ed.setOverride('hull.repair_team_count', 5);
    ed.setOverride('helm_console.max_speed', 80.0);
    ed.setOverride('radar_appearance.colour', [1.0, 0.0, 0.0]);

    // Clear one override
    ed.clearOverride('hull.repair_team_count');

    // Summary should have 2 entries
    const summary = ed.getOverridesSummary();
    expect(summary).toHaveLength(2);
    expect(summary.find((e) => e.path === 'hull.repair_team_count')).toBeUndefined();

    // Resolved view: cleared field falls back to template
    const resolved = ed.getResolvedView();
    expect(resolved.hull.repair_team_count).toBe(PLAYER_SHIP.hull.repair_team_count);
    expect(resolved.helm_console.max_speed).toBe(80.0);
    expect(resolved.radar_appearance.colour).toEqual([1.0, 0.0, 0.0]);

    // TOML serialises remaining overrides
    const toml = ed.toOverridesToml();
    const reparsed = parse(toml);
    expect(reparsed.helm_console.max_speed).toBe(80.0);
    expect(reparsed.radar_appearance.colour).toEqual([1.0, 0.0, 0.0]);
    expect(reparsed.hull).toBeUndefined();
  });

  it('clear all overrides leaves empty TOML and resolved equals template', () => {
    const ed = new OverrideEditor(PLAYER_SHIP);
    ed.setOverride('radar_appearance.colour', [1.0, 0.0, 0.0]);
    ed.setOverride('helm_console.max_speed', 80.0);
    ed.clearOverride('radar_appearance.colour');
    ed.clearOverride('helm_console.max_speed');
    expect(ed.toOverridesToml()).toBe('');
    expect(ed.getResolvedView()).toEqual(PLAYER_SHIP);
  });

  it('novel subfield survives the full cycle', () => {
    const ed = new OverrideEditor(PLAYER_SHIP);
    ed.setOverride('exotic_drive.warp_factor', 9);
    expect(ed.getResolvedView().exotic_drive.warp_factor).toBe(9);
    const reparsed = parse(ed.toOverridesToml());
    expect(reparsed.exotic_drive.warp_factor).toBe(9);
    ed.clearOverride('exotic_drive.warp_factor');
    expect(ed.getResolvedView().exotic_drive).toBeUndefined();
  });
});
