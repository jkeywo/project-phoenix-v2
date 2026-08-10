import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { CanvasManager, buildSpawnRenderView } from '../canvas.js';
import { entityCache } from '../entity-cache.js';
import { stringifyWorldToml, parseWorldToml } from '../world-toml.js';

/**
 * Issue #993 — `renderSpawn` must NOT bake template/fragment appearance fields
 * onto the persisted spawn.
 *
 * The spawn objects in `layer.toml.entity` are the world layer's saved data
 * model (a save serialises them straight through `stringifyWorldToml`). Before
 * this fix, `renderSpawn` merged the resolved template's `tags` /
 * `radar_appearance` / `collider` / `shape` / `effects` / `colour` (and the
 * `asteroid_field`→torus synthesis) onto the spawn whenever it lacked them, so a
 * save froze a stale copy of those fields into the world TOML and editing the
 * template later left every saved spawn stale.
 *
 * These tests pin the corrected contract: the merge lands on a throwaway view
 * used only for drawing, the persisted spawn is never mutated, and a
 * render → serialize round-trip carries only spawn-authored data +
 * `template_path` + `override`. They fail if the merge is ever reverted to
 * mutating the spawn in place (the mutation gap #910's re-review flagged).
 */

// A resolved template doc as `getEntityConfig` would return post-#910 — for a
// composed hull these very fields are fragment-inherited, so the same test
// covers the widened blast radius without needing the resolver itself.
const TEMPLATE_PATH = 'assets/entities/raider_composed.toml';
const RESOLVED_TEMPLATE = {
  tags: ['ship', 'npc', 'enemy'],
  radar_appearance: { colour: [1.0, 0.2, 0.2], size: 4.0, icon: 'destroyer' },
  collider: { radius: 9.0 },
  shape: { type: 'sphere', radius: 120.0 },
  effects: { damage_zone: { damage_per_second: 5.0 } },
  colour: [0.2, 0.4, 0.6],
};

const TEMPLATE_FIELDS = ['tags', 'radar_appearance', 'collider', 'shape', 'effects', 'colour'];

// ── Minimal Konva stub ──────────────────────────────────────────────────────
// canvas.js references a bare global `Konva`; renderSpawn constructs Groups,
// Rings, Circles, Text, Lines and Rects and calls `.add()` / `.on()`.
function makeKonvaStub() {
  class Node {
    constructor(cfg = {}) { this.cfg = cfg; this.children = []; this.handlers = {}; }
    add(child) { this.children.push(child); }
    on(evt, fn) { this.handlers[evt] = fn; }
    x() { return this.cfg.x ?? 0; }
    y() { return this.cfg.y ?? 0; }
    destroy() {}
  }
  return {
    Group: Node, Ring: Node, Circle: Node, Text: Node,
    Line: Node, Rect: Node, Arrow: Node, Stage: Node, Layer: Node,
  };
}

function makeCanvasManager() {
  const undoController = { snapshotForUndo() {} };
  const cm = new CanvasManager(
    { getLayers: () => [] }, // layerManager (unused by renderSpawn)
    () => {}, () => {}, () => {}, () => {},
    undoController,
  );
  cm.scale = 1;
  return cm;
}

describe('renderSpawn does not bake template/fragment fields (issue #993)', () => {
  let cm;
  let container;

  beforeEach(() => {
    entityCache.clear();
    entityCache.set(TEMPLATE_PATH, RESOLVED_TEMPLATE);
    globalThis.Konva = makeKonvaStub();
    cm = makeCanvasManager();
    container = new globalThis.Konva.Group();
  });

  afterEach(() => {
    entityCache.clear();
    delete globalThis.Konva;
  });

  it('buildSpawnRenderView returns a fresh object carrying the template appearance', () => {
    const spawn = { name: 'r1', template_path: TEMPLATE_PATH, transform: { position: [0, 0, 0] } };
    const view = buildSpawnRenderView(spawn, RESOLVED_TEMPLATE);

    // The view drives rendering: it must carry every template appearance field…
    expect(view.tags).toEqual(RESOLVED_TEMPLATE.tags);
    expect(view.radar_appearance).toEqual(RESOLVED_TEMPLATE.radar_appearance);
    expect(view.collider).toEqual(RESOLVED_TEMPLATE.collider);
    expect(view.shape).toEqual(RESOLVED_TEMPLATE.shape);
    expect(view.effects).toEqual(RESOLVED_TEMPLATE.effects);
    expect(view.colour).toEqual(RESOLVED_TEMPLATE.colour);
    // …and preserve the spawn's own authored data.
    expect(view.name).toBe('r1');
    expect(view.template_path).toBe(TEMPLATE_PATH);

    // …while NOT being the spawn itself (a throwaway).
    expect(view).not.toBe(spawn);
    expect(spawn.tags).toBeUndefined();
    expect(spawn.shape).toBeUndefined();
  });

  it('synthesizes a torus shape from an asteroid_field block onto the view only', () => {
    const spawn = { name: 'belt', template_path: 'assets/entities/belt.toml' };
    const view = buildSpawnRenderView(spawn, {
      asteroid_field: { inner_radius: 300, outer_radius: 500 },
    });
    expect(view.shape).toEqual({ type: 'torus', inner_radius: 300, outer_radius: 500 });
    expect(spawn.shape).toBeUndefined();
  });

  it('an explicit template shape wins over the asteroid_field synthesis', () => {
    const view = buildSpawnRenderView(
      { template_path: 'x' },
      { shape: { type: 'box', half_extents: [1, 2, 3] }, asteroid_field: { inner_radius: 1, outer_radius: 2 } },
    );
    expect(view.shape).toEqual({ type: 'box', half_extents: [1, 2, 3] });
  });

  it('the spawn keeps a field it authored itself (template does not override it)', () => {
    const spawn = { template_path: TEMPLATE_PATH, tags: ['authored'] };
    const view = buildSpawnRenderView(spawn, RESOLVED_TEMPLATE);
    expect(view.tags).toEqual(['authored']);
    expect(spawn.tags).toEqual(['authored']);
  });

  it('render leaves the persisted spawn free of every template-derived field', () => {
    const spawn = {
      name: 'raider_alpha',
      template_path: TEMPLATE_PATH,
      transform: { position: [10, 0, -20] },
      override: { hull: { max: 50 } },
    };
    const keysBefore = Object.keys(spawn).sort();

    cm.renderSpawn(spawn, { toml: { entity: [spawn] }, isDirty: false }, container, []);

    // The mutation gap: none of the template fields may appear on the spawn.
    for (const field of TEMPLATE_FIELDS) {
      expect(field in spawn, `spawn must not carry template-derived "${field}"`).toBe(false);
    }
    // Spawn-authored keys are untouched — no additions, no removals.
    expect(Object.keys(spawn).sort()).toEqual(keysBefore);
    expect(spawn.override).toEqual({ hull: { max: 50 } });
    // Rendering still happened (the throwaway view produced marker geometry).
    expect(container.children.length).toBeGreaterThan(0);
  });

  it('render → serialize round-trip freezes no template-derived field into the world TOML', () => {
    const spawn = {
      name: 'raider_alpha',
      template_path: TEMPLATE_PATH,
      transform: { position: [10, 0, -20] },
      override: { hull: { max: 50 } },
    };
    const world = { global: { seed: 1 }, anchors: {}, entity: [spawn] };

    // Render (canvas draw) BEFORE the save — this is the round-trip that used
    // to bake the template fields onto the spawn.
    cm.renderSpawn(spawn, { toml: world, isDirty: false }, container, []);

    const reparsed = parseWorldToml(stringifyWorldToml(world));
    const persisted = reparsed.entity[0];

    for (const field of TEMPLATE_FIELDS) {
      expect(field in persisted, `world TOML must not carry baked "${field}"`).toBe(false);
    }
    // Only spawn-authored data + template_path + override survive.
    expect(persisted.template_path).toBe(TEMPLATE_PATH);
    expect(persisted.name).toBe('raider_alpha');
    expect(persisted.override).toEqual({ hull: { max: 50 } });
    expect(persisted.transform).toEqual({ position: [10, 0, -20] });
    expect(Object.keys(persisted).sort()).toEqual(
      ['name', 'override', 'template_path', 'transform'].sort(),
    );
  });

  it('a non-template spawn round-trips its own authored fields unchanged', () => {
    // A region authored directly in the world (no template_path): its own
    // shape/colour/effects are spawn-authored and MUST persist untouched.
    const spawn = {
      name: 'nebula_1',
      transform: { position: [5, 0, 5] },
      shape: { type: 'sphere', radius: 200 },
      colour: [0.4, 0.6, 1.0],
      effects: { slow_zone: { factor: 0.5 } },
    };
    const before = JSON.parse(JSON.stringify(spawn));
    const world = { global: { seed: 1 }, anchors: {}, entity: [spawn] };

    cm.renderSpawn(spawn, { toml: world, isDirty: false }, container, []);

    // Render did not mutate the authored spawn at all.
    expect(spawn).toEqual(before);
    const persisted = parseWorldToml(stringifyWorldToml(world)).entity[0];
    expect(persisted.shape).toEqual({ type: 'sphere', radius: 200 });
    expect(persisted.colour).toEqual([0.4, 0.6, 1.0]);
    expect(persisted.effects).toEqual({ slow_zone: { factor: 0.5 } });
  });
});
