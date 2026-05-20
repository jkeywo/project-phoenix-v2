import { describe, it, expect, beforeEach } from 'vitest';
import { LayerManager } from '../layer-manager.js';

// ── Shared fixtures ───────────────────────────────────────────────────────────

const ROOT_PATH = 'assets/worlds/default.toml';
const CHILD_A_PATH = 'assets/worlds/patrol.toml';
const CHILD_B_PATH = 'assets/worlds/side-mission.toml';

/** Minimal root world with two extra_worlds children. */
const rootContent = {
  extra_worlds: [CHILD_A_PATH, CHILD_B_PATH],
  global: { seed: 42 },
  anchors: { starbase_alpha: [500.0, 0.0, 0.0] },
  entity: [
    { template_path: 'assets/entities/star_sun.toml', position: [0.0, 0.0, 0.0] },
  ],
};

const childAContent = {
  global: { seed: 1 },
  anchors: { patrol_alpha: [300.0, 0.0, -300.0] },
  entity: [
    { template_path: 'assets/entities/pirate_raider.toml', name: 'raider_alpha', anchor: 'patrol_alpha' },
  ],
};

const childBContent = {
  global: { seed: 2 },
  anchors: { side_base: [100.0, 0.0, 100.0] },
  entity: [],
};

const extraWorldContents = {
  [CHILD_A_PATH]: childAContent,
  [CHILD_B_PATH]: childBContent,
};

// ── openRoot ──────────────────────────────────────────────────────────────────

describe('LayerManager.openRoot', () => {
  it('creates a LayerManager with 3 layers (root + 2 children)', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    const tree = lm.getLayerTree();
    expect(tree).toHaveLength(1);                       // one root node
    expect(tree[0].children).toHaveLength(2);          // two children
  });

  it('root layer path matches rootPath', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    const tree = lm.getLayerTree();
    expect(tree[0].path).toBe(ROOT_PATH);
  });

  it('child paths match extra_worlds entries', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    const tree = lm.getLayerTree();
    const childPaths = tree[0].children.map(c => c.path);
    expect(childPaths).toContain(CHILD_A_PATH);
    expect(childPaths).toContain(CHILD_B_PATH);
  });

  it('root is active by default', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    const tree = lm.getLayerTree();
    expect(tree[0].active).toBe(true);
  });

  it('children are not active by default', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    const tree = lm.getLayerTree();
    for (const child of tree[0].children) {
      expect(child.active).toBe(false);
    }
  });

  it('all layers start with dirty = false', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    const tree = lm.getLayerTree();
    expect(tree[0].dirty).toBe(false);
    for (const child of tree[0].children) {
      expect(child.dirty).toBe(false);
    }
  });

  it('works when extra_worlds is empty', () => {
    const noChildren = { global: { seed: 1 }, anchors: {} };
    const lm = LayerManager.openRoot(ROOT_PATH, noChildren, {});
    const tree = lm.getLayerTree();
    expect(tree[0].children).toHaveLength(0);
  });

  it('works when extra_worlds key is absent', () => {
    const noExtraWorlds = { global: { seed: 1 }, anchors: {} };
    const lm = LayerManager.openRoot(ROOT_PATH, noExtraWorlds, {});
    const tree = lm.getLayerTree();
    expect(tree[0].children).toHaveLength(0);
  });

  it('gracefully skips extra_worlds paths not present in extraWorldContents', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, {
      [CHILD_A_PATH]: childAContent,
      // CHILD_B_PATH intentionally absent
    });
    const tree = lm.getLayerTree();
    // Root still has the two child paths registered in children array,
    // but the tree node only includes layers that were actually loaded.
    const childPaths = tree[0].children.map(c => c.path);
    expect(childPaths).toContain(CHILD_A_PATH);
    expect(childPaths).not.toContain(CHILD_B_PATH);
  });

  it('does not mutate the passed-in rootContent (deep clone)', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    const ws = lm.getWorldState(ROOT_PATH);
    ws.global.seed = 999;
    expect(rootContent.global.seed).toBe(42);
  });
});

// ── getLayerTree ─────────────────────────────────────────────────────────────

describe('getLayerTree', () => {
  it('returns [] when no layers are loaded', () => {
    const lm = new LayerManager();
    expect(lm.getLayerTree()).toEqual([]);
  });

  it('each node has path, dirty, active, children', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    const [root] = lm.getLayerTree();
    expect(root).toHaveProperty('path');
    expect(root).toHaveProperty('dirty');
    expect(root).toHaveProperty('active');
    expect(root).toHaveProperty('children');
  });
});

// ── setActiveLayer ────────────────────────────────────────────────────────────

describe('setActiveLayer', () => {
  it('activates the named child and deactivates root', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents)
      .setActiveLayer(CHILD_A_PATH);
    const tree = lm.getLayerTree();
    expect(tree[0].active).toBe(false);
    const childA = tree[0].children.find(c => c.path === CHILD_A_PATH);
    expect(childA.active).toBe(true);
  });

  it('only one layer is active at a time', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents)
      .setActiveLayer(CHILD_A_PATH);
    const countActive = (nodes) => nodes.reduce((n, node) => {
      return n + (node.active ? 1 : 0) + countActive(node.children);
    }, 0);
    expect(countActive(lm.getLayerTree())).toBe(1);
  });

  it('returns original manager unchanged when path not found', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    const lm2 = lm.setActiveLayer('nonexistent/path.toml');
    expect(lm2.getLayerTree()[0].active).toBe(true);
  });

  it('returns a new LayerManager instance (immutable)', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    const lm2 = lm.setActiveLayer(CHILD_A_PATH);
    expect(lm2).not.toBe(lm);
    // Original still has root active
    expect(lm.getLayerTree()[0].active).toBe(true);
  });

  it('switching back to root re-activates root', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents)
      .setActiveLayer(CHILD_A_PATH)
      .setActiveLayer(ROOT_PATH);
    expect(lm.getLayerTree()[0].active).toBe(true);
  });
});

// ── markDirty ─────────────────────────────────────────────────────────────────

describe('markDirty', () => {
  it('marks the specified layer dirty', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents)
      .markDirty(ROOT_PATH);
    expect(lm.getLayerTree()[0].dirty).toBe(true);
  });

  it('does not dirty other layers', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents)
      .markDirty(ROOT_PATH);
    for (const child of lm.getLayerTree()[0].children) {
      expect(child.dirty).toBe(false);
    }
  });

  it('is a no-op for unknown path (does not throw)', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    expect(() => lm.markDirty('unknown.toml')).not.toThrow();
  });

  it('returns a new LayerManager instance', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    expect(lm.markDirty(ROOT_PATH)).not.toBe(lm);
  });
});

// ── addSpawn ──────────────────────────────────────────────────────────────────

describe('addSpawn', () => {
  const newSpawn = { template_path: 'assets/entities/player_ship.toml', position: [0.0, 0.0, 0.0] };

  it('adds spawn to active layer (root by default)', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents)
      .addSpawn(newSpawn);
    const ws = lm.getWorldState(ROOT_PATH);
    const found = ws.entity.find(e => e.template_path === newSpawn.template_path && e.position);
    expect(found).toBeDefined();
  });

  it('marks the active layer dirty after addSpawn', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents)
      .addSpawn(newSpawn);
    expect(lm.getLayerTree()[0].dirty).toBe(true);
  });

  it('lands spawn in child layer when child is active', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents)
      .setActiveLayer(CHILD_A_PATH)
      .addSpawn(newSpawn);
    const childWs = lm.getWorldState(CHILD_A_PATH);
    const found = childWs.entity.find(e => e.template_path === newSpawn.template_path);
    expect(found).toBeDefined();
  });

  it('does NOT add spawn to inactive layers', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents)
      .setActiveLayer(CHILD_A_PATH)
      .addSpawn(newSpawn);
    // Root (inactive) entity count should be unchanged
    const rootWs = lm.getWorldState(ROOT_PATH);
    expect(rootWs.entity).toHaveLength(rootContent.entity.length);
  });

  it('works when entity array is absent in active layer', () => {
    const noEntities = { global: { seed: 1 }, anchors: {} };
    const lm = LayerManager.openRoot(ROOT_PATH, noEntities, {}).addSpawn(newSpawn);
    const ws = lm.getWorldState(ROOT_PATH);
    expect(ws.entity).toHaveLength(1);
    expect(ws.entity[0]).toEqual(newSpawn);
  });

  it('returns a new LayerManager (immutable)', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    const lm2 = lm.addSpawn(newSpawn);
    expect(lm2).not.toBe(lm);
  });
});

// ── addAnchor ─────────────────────────────────────────────────────────────────

describe('addAnchor', () => {
  it('adds anchor to active layer world state', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents)
      .addAnchor('new_base', [100.0, 0.0, 200.0]);
    const ws = lm.getWorldState(ROOT_PATH);
    expect(ws.anchors.new_base).toEqual([100.0, 0.0, 200.0]);
  });

  it('marks the active layer dirty', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents)
      .addAnchor('new_base', [0.0, 0.0, 0.0]);
    expect(lm.getLayerTree()[0].dirty).toBe(true);
  });

  it('lands anchor in child layer when child is active', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents)
      .setActiveLayer(CHILD_B_PATH)
      .addAnchor('side_outpost', [50.0, 0.0, 50.0]);
    const ws = lm.getWorldState(CHILD_B_PATH);
    expect(ws.anchors.side_outpost).toEqual([50.0, 0.0, 50.0]);
  });

  it('does not add anchor to inactive layers', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents)
      .setActiveLayer(CHILD_A_PATH)
      .addAnchor('new_anchor', [1.0, 0.0, 2.0]);
    const rootWs = lm.getWorldState(ROOT_PATH);
    expect(rootWs.anchors.new_anchor).toBeUndefined();
  });

  it('works when anchors object is absent in active layer', () => {
    const noAnchors = { global: { seed: 1 } };
    const lm = LayerManager.openRoot(ROOT_PATH, noAnchors, {})
      .addAnchor('genesis', [0.0, 0.0, 0.0]);
    const ws = lm.getWorldState(ROOT_PATH);
    expect(ws.anchors.genesis).toEqual([0.0, 0.0, 0.0]);
  });
});

// ── getWorldState ─────────────────────────────────────────────────────────────

describe('getWorldState', () => {
  it('returns the world state for an existing layer', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    const ws = lm.getWorldState(ROOT_PATH);
    expect(ws.global.seed).toBe(42);
  });

  it('returns the child world state correctly', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    const ws = lm.getWorldState(CHILD_A_PATH);
    expect(ws.global.seed).toBe(1);
    expect(ws.anchors.patrol_alpha).toEqual([300.0, 0.0, -300.0]);
  });

  it('returns null for an unknown path', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    expect(lm.getWorldState('nonexistent.toml')).toBeNull();
  });
});

// ── Integration: primary acceptance-criteria scenario ─────────────────────────
// Load fixture root with two extra_worlds → tree has 3 entries
// → setActiveLayer(child) → new spawn lands in child's entity list

describe('integration: load root with two extra_worlds', () => {
  it('tree has 3 entries total (1 root + 2 children)', () => {
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents);
    const tree = lm.getLayerTree();
    // Count all nodes
    const countNodes = (nodes) => nodes.reduce((n, node) => n + 1 + countNodes(node.children), 0);
    expect(countNodes(tree)).toBe(3);
  });

  it('setActiveLayer(child) → new spawn lands in child entity list', () => {
    const newSpawn = { template_path: 'assets/entities/pirate_raider.toml', name: 'new_raider', position: [0.0, 0.0, 0.0] };
    const initialChildEntityCount = childAContent.entity.length;

    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents)
      .setActiveLayer(CHILD_A_PATH)
      .addSpawn(newSpawn);

    const childWs = lm.getWorldState(CHILD_A_PATH);
    expect(childWs.entity).toHaveLength(initialChildEntityCount + 1);
    expect(childWs.entity[childWs.entity.length - 1]).toEqual(newSpawn);

    // Root is unaffected
    const rootWs = lm.getWorldState(ROOT_PATH);
    expect(rootWs.entity).toHaveLength(rootContent.entity.length);
  });

  it('dirty indicator updates correctly per layer', () => {
    const spawn = { template_path: 'assets/entities/star_sun.toml', position: [0.0, 0.0, 0.0] };
    const lm = LayerManager.openRoot(ROOT_PATH, rootContent, extraWorldContents)
      .setActiveLayer(CHILD_A_PATH)
      .addSpawn(spawn);

    const tree = lm.getLayerTree();
    const childANode = tree[0].children.find(c => c.path === CHILD_A_PATH);
    const childBNode = tree[0].children.find(c => c.path === CHILD_B_PATH);

    expect(tree[0].dirty).toBe(false);        // root untouched
    expect(childANode.dirty).toBe(true);       // child A modified
    expect(childBNode.dirty).toBe(false);      // child B untouched
  });
});
