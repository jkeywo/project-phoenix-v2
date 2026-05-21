import { describe, it, expect } from 'vitest';
import { OverrideEditor } from '../override-editor.js';
import { CrossReferenceIndex } from '../cross-references.js';
import { getWorldContentData } from '../world-content-panel.js';
import { parseWorldToml, stringifyWorldToml } from '../world-toml.js';

/**
 * Integration test for the Slice 3 select → override → clear cycle.
 *
 * Mirrors what override-view.js does at the data layer (no DOM): build an
 * editor from the spawn's template, replay existing overrides, mutate via
 * setOverride/clearOverride, write back to `spawn.override`, and confirm
 * the world TOML round-trips through smol-toml unchanged.
 *
 * Also confirms CrossReferenceIndex + getWorldContentData remain in sync
 * before and after the mutation — the World Content panel must reflect
 * the same entity at all times.
 */

// Helpers copied from override-view.js (kept private there to avoid
// growing the module API surface).  They are part of the contract this
// test pins.
function isPlainObject(v) {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}
function flattenLeaves(obj, prefix = '') {
  const out = [];
  if (!isPlainObject(obj)) return out;
  for (const [k, v] of Object.entries(obj)) {
    const p = prefix ? `${prefix}.${k}` : k;
    if (isPlainObject(v)) out.push(...flattenLeaves(v, p));
    else out.push({ path: p, value: v });
  }
  return out;
}

function makeWorld() {
  return {
    global: { seed: 42 },
    anchors: { patrol_alpha: [100, 0, 0] },
    entity: [{
      template_path: 'assets/entities/pirate_raider.toml',
      name: 'raider_alpha',
      anchor: 'patrol_alpha',
    }],
    trigger: [{
      condition: 'on_destroyed',
      entity: 'raider_alpha',
      action: [{ type: 'add_objective', id: 'win', text: 'destroy them' }],
    }],
  };
}

function makeTemplate() {
  // Minimal pirate-raider template; only the fields the test exercises.
  return {
    hull: { max: 100, current: 100 },
    helm_console: { max_speed: 30 },
    radar_appearance: { colour: [1.0, 0.0, 0.0], shape: 'triangle' },
    tags: ['hostile', 'ship'],
  };
}

describe('Slice 3 integration: select → override → clear cycle', () => {
  it('full cycle: select entity, attach override, summary updates, clear restores template', () => {
    const world = makeWorld();
    const template = makeTemplate();
    const spawn = world.entity[0];

    // ── 1. Select: user clicks raider_alpha; sidebar opens.
    //    Build the editor and replay any existing overrides (none here).
    let editor = new OverrideEditor(template);
    for (const { path, value } of flattenLeaves(spawn.override ?? {})) {
      editor.setOverride(path, value);
    }

    // No overrides yet — resolved view equals template.
    expect(editor.getResolvedView()).toEqual(template);
    expect(editor.getOverridesSummary()).toEqual([]);
    expect('override' in spawn).toBe(false);

    // ── 2. Edit a primitive: click hull.max, type 80.
    editor.setOverride('hull.max', 80);
    spawn.override = editor.getOverrides();

    expect(spawn.override).toEqual({ hull: { max: 80 } });
    expect(editor.getResolvedView().hull.max).toBe(80);
    expect(editor.getResolvedView().hull.current).toBe(100); // template field preserved
    expect(editor.getOverridesSummary())
      .toEqual([{ path: 'hull.max', value: 80 }]);

    // ── 3. Edit an array (REPLACE-on-merge per audit §10).
    editor.setOverride('radar_appearance.colour', [0.8, 0, 0]);
    spawn.override = editor.getOverrides();

    expect(spawn.override.radar_appearance.colour).toEqual([0.8, 0, 0]);
    expect(editor.getResolvedView().radar_appearance.shape).toBe('triangle');
    expect(editor.getOverridesSummary()).toHaveLength(2);

    // ── 4. Round-trip the entire world through TOML — overrides survive.
    const text = stringifyWorldToml(world);
    const reparsed = parseWorldToml(text);
    expect(reparsed.entity[0].override).toEqual({
      hull: { max: 80 },
      radar_appearance: { colour: [0.8, 0, 0] },
    });

    // ── 5. Clear one override: hull.max.
    editor.clearOverride('hull.max');
    spawn.override = editor.getOverrides();
    if (Object.keys(spawn.override).length === 0) delete spawn.override;

    expect(spawn.override).toEqual({
      radar_appearance: { colour: [0.8, 0, 0] },
    });
    expect(editor.getResolvedView().hull.max).toBe(100); // back to template

    // ── 6. Clear the last override: the override key is removed entirely.
    editor.clearOverride('radar_appearance.colour');
    spawn.override = editor.getOverrides();
    if (Object.keys(spawn.override).length === 0) delete spawn.override;

    expect('override' in spawn).toBe(false);
    expect(editor.getResolvedView()).toEqual(template);
  });

  it('CrossReferenceIndex + WorldContentData stay coherent across the cycle', () => {
    const world = makeWorld();
    const layers = [{ path: 'assets/worlds/test.toml', worldState: world }];

    const index = new CrossReferenceIndex();
    index.indexLayers(layers);

    // Initial: raider_alpha is referenced once (the on_destroyed trigger).
    const before = getWorldContentData(world, index, 'assets/worlds/test.toml');
    expect(before.namedEntities).toEqual([{
      name: 'raider_alpha',
      template_path: 'assets/entities/pirate_raider.toml',
      refCount: 1,
    }]);
    expect(before.objectives).toEqual([{
      id: 'win', text: 'destroy them', mandatory: undefined, refCount: 1,
    }]);

    // Attach an override to the spawn — does not touch references.
    const editor = new OverrideEditor(makeTemplate());
    editor.setOverride('hull.max', 80);
    world.entity[0].override = editor.getOverrides();
    index.indexLayers(layers); // mimic renderAll() rebuild

    const after = getWorldContentData(world, index, 'assets/worlds/test.toml');
    expect(after.namedEntities[0].refCount).toBe(1);
    expect(after.objectives[0].refCount).toBe(1);
    // The named-entity row still resolves to the same entity for click-
    // to-highlight; the World Content tree doesn't show overrides itself.
    expect(after.namedEntities[0].name).toBe('raider_alpha');
  });

  it('replaying an existing override on selection produces the same resolved view', () => {
    // Simulates re-opening a world that already carries overrides on disk.
    const world = makeWorld();
    world.entity[0].override = { hull: { max: 80 } };
    const template = makeTemplate();
    const spawn = world.entity[0];

    const editor = new OverrideEditor(template);
    for (const { path, value } of flattenLeaves(spawn.override ?? {})) {
      editor.setOverride(path, value);
    }
    expect(editor.getResolvedView().hull.max).toBe(80);
    expect(editor.getOverridesSummary())
      .toEqual([{ path: 'hull.max', value: 80 }]);
  });
});
