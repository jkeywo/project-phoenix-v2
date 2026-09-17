import { describe, expect, it } from 'vitest';
import {
  closeWorkshopPanel, defaultWorkshopLayout, dockWorkshopPanel, floatWorkshopPanel,
  moveWorkshopFloat, normalizeWorkshopLayout, reopenWorkshopPanel, workshopLayoutModel,
} from '../../gui/workshop-layout-model.js';

const panels = state => JSON.stringify(state.root) + JSON.stringify(state.floats);

describe('Workshop layout model', () => {
  it('tabs, splits, floats, moves, closes, reopens and resets all panels', () => {
    let state = defaultWorkshopLayout();
    state = dockWorkshopPanel(state, 'inspector', 'source', 'tab');
    expect(state.root.children[1]).toMatchObject({
      tabs: ['source', 'findings', 'feedback', 'model-preview', 'inspector'], active: 'inspector' });
    state = dockWorkshopPanel(state, 'files', 'source', 'bottom');
    expect(panels(state)).toContain('"axis":"vertical"');
    state = floatWorkshopPanel(state, 'inspector', { x: 8, y: 9, width: 300, height: 200 });
    expect(state.floats[0]).toMatchObject({ panel: 'inspector', x: 8, y: 9 });
    state = moveWorkshopFloat(state, 'inspector', 40, 50);
    expect(state.floats[0]).toMatchObject({ x: 40, y: 50 });
    state = closeWorkshopPanel(state, 'inspector');
    expect(state.closed).toContain('inspector');
    state = reopenWorkshopPanel(state, 'inspector');
    expect(state.closed).not.toContain('inspector');
    expect(panels(state)).toContain('inspector');
    expect(defaultWorkshopLayout().root.children).toHaveLength(3);
    expect(defaultWorkshopLayout().root.children[2]).toMatchObject({
      tabs: ['inspector', 'add', 'recovery', 'settings', 'models', 'sound'], active: 'inspector',
    });
    expect(defaultWorkshopLayout().root.children[1]).toMatchObject({
      tabs: ['source', 'findings', 'feedback', 'model-preview'], active: 'source',
    });
  });

  it('repairs malformed, duplicate and obsolete layouts without losing panels', () => {
    const repaired = normalizeWorkshopLayout({ version: 2, selected: 'future', closed: ['future'], floats: [], root: {
      type: 'split', axis: 'sideways', children: [{ type: 'tabs', tabs: ['source', 'source', 'future'] }],
    } });
    expect(repaired).toEqual(defaultWorkshopLayout());
    expect(normalizeWorkshopLayout({ version: 99 })).toEqual(defaultWorkshopLayout());
  });

  it('migrates three-panel placement and registers lifecycle panels without recovery data', () => {
    const migrated = normalizeWorkshopLayout({
      version: 1,
      root: { type: 'split', axis: 'horizontal', sizes: [1, 2], children: [
        { type: 'tabs', tabs: ['source'], active: 'source' },
        { type: 'tabs', tabs: ['files', 'inspector'], active: 'files' },
      ] },
      floats: [], closed: [], selected: 'source', recovery: { draft: 'must not persist' },
    });
    expect(migrated.version).toBe(4);
    expect(migrated.selected).toBe('source');
    for (const panel of ['dependencies', 'findings', 'feedback', 'settings', 'models', 'model-preview', 'sound']) {
      expect(panels(migrated)).toContain(`"${panel}"`);
    }
    expect(migrated).not.toHaveProperty('recovery');
  });

  it('rehomes crafted legacy lifecycle panels without duplicates', () => {
    const migrated = normalizeWorkshopLayout({
      version: 1,
      root: { type: 'split', axis: 'horizontal', sizes: [10, 30, 60], children: [
        { type: 'tabs', tabs: ['files', 'add'], active: 'add' },
        { type: 'tabs', tabs: ['source'], active: 'source' },
        { type: 'tabs', tabs: ['inspector'], active: 'inspector' },
      ] },
      floats: [{ panel: 'recovery', x: 7, y: 9, width: 300, height: 200 }],
      closed: [], selected: 'source',
    });

    expect(migrated.version).toBe(4);
    const placed = [];
    const collect = node => {
      if (node?.type === 'tabs') placed.push(...node.tabs);
      else if (node?.type === 'split') node.children.forEach(collect);
    };
    collect(migrated.root);
    placed.push(...migrated.floats.map(entry => entry.panel), ...migrated.closed);
    for (const panel of ['files', 'source', 'inspector', 'add', 'recovery', 'dependencies', 'findings', 'feedback',
      'settings', 'models', 'model-preview', 'sound']) {
      expect(placed.filter(candidate => candidate === panel)).toHaveLength(1);
    }
  });

  it.each([1, 2])('preserves a v%s layout when preferred later targets are floating', version => {
    const floats = [
      { panel: 'files', x: 13, y: 17, width: 301, height: 211 },
      { panel: 'source', x: 41, y: 47, width: 503, height: 307 },
    ];
    const migrated = normalizeWorkshopLayout({
      version,
      root: { type: 'split', axis: 'vertical', sizes: [17, 83], children: [
        { type: 'tabs', tabs: ['inspector'], active: 'inspector' },
        { type: 'tabs', tabs: ['recovery'], active: 'recovery' },
      ] },
      floats, closed: ['add'], selected: 'source',
    });

    expect(migrated).toEqual({
      version: 4,
      root: { type: 'split', axis: 'vertical', sizes: [17, 83], children: [
        { type: 'tabs',
          tabs: ['inspector', 'dependencies', 'findings', 'feedback', 'settings', 'models', 'model-preview', 'sound'],
          active: 'inspector' },
        { type: 'tabs', tabs: ['recovery'], active: 'recovery' },
      ] },
      floats, closed: ['add'], selected: 'source',
    });
  });

  it('deduplicates globally before applying collection limits', () => {
    const repaired = normalizeWorkshopLayout({
      version: 2,
      root: null,
      floats: [
        { panel: 'source' }, { panel: 'source' }, { panel: 'source' },
        { panel: 'inspector', x: 30, y: 40 }, { panel: 'files' },
      ],
      closed: ['source', 'inspector', 'files'], selected: 'inspector',
    });
    expect(repaired.version).toBe(4);
    expect(repaired.selected).toBe('inspector');
    expect(repaired.closed).toEqual(['add', 'recovery']);
    for (const panel of ['source', 'inspector', 'files', 'dependencies', 'findings', 'feedback', 'settings',
      'models', 'model-preview', 'sound']) {
      expect(panels(repaired)).toContain(`"${panel}"`);
    }
  });

  it('reopens into a new dock without disturbing existing floats', () => {
    let state = defaultWorkshopLayout();
    state = floatWorkshopPanel(state, 'files', { x: 18, y: 24, width: 300, height: 200 });
    state = floatWorkshopPanel(state, 'source', { x: 48, y: 54, width: 320, height: 220 });
    state = closeWorkshopPanel(state, 'inspector');
    const floats = state.floats;

    state = reopenWorkshopPanel(state, 'inspector');

    expect(panels(state)).toContain('"inspector"');
    expect(state.floats).toEqual(floats);
  });

  it('stagger-defaults newly floated panels and preserves explicit placement', () => {
    let state = floatWorkshopPanel(defaultWorkshopLayout(), 'files');
    state = floatWorkshopPanel(state, 'source');
    state = floatWorkshopPanel(state, 'inspector', { x: 7, y: 9 });
    expect(state.floats.map(({ x, y }) => [x, y])).toEqual([[24, 24], [52, 52], [7, 9]]);
  });

  it('defaults missing and malformed roots but preserves intentionally all-closed legacy layouts', () => {
    const closed = ['files', 'source', 'inspector', 'add', 'recovery'];
    expect(normalizeWorkshopLayout({ version: 2, floats: [], closed }))
      .toEqual(defaultWorkshopLayout());
    expect(normalizeWorkshopLayout({
      version: 2, root: { type: 'unknown' }, floats: [], closed,
    })).toEqual(defaultWorkshopLayout());
    for (const version of [1, 2]) {
      expect(normalizeWorkshopLayout({
        version, root: null, floats: [], closed, selected: 'source',
      })).toEqual({ version: 4, root: null, floats: [],
        closed: [...closed, 'dependencies', 'findings', 'feedback', 'settings', 'models', 'model-preview', 'sound'],
        selected: 'files' });
    }
  });

  it('defaults an untrusted layout whose split nesting exceeds the panel bound', () => {
    let root = { type: 'tabs', tabs: ['source'], active: 'source' };
    for (let depth = 0; depth < 100; depth += 1) {
      root = { type: 'split', axis: 'horizontal', sizes: [1], children: [root] };
    }
    expect(normalizeWorkshopLayout({
      version: 2, root, floats: [], closed: ['files', 'inspector', 'add', 'recovery'], selected: 'source',
    })).toEqual(defaultWorkshopLayout());
  });

  it('registers the media panels on a stored v3 layout without reopening a closed v3 panel', () => {
    const migrated = normalizeWorkshopLayout({
      version: 3,
      root: { type: 'split', axis: 'horizontal', sizes: [22, 56, 22], children: [
        { type: 'tabs', tabs: ['files', 'dependencies'], active: 'files' },
        { type: 'tabs', tabs: ['source', 'findings'], active: 'source' },
        { type: 'tabs', tabs: ['inspector', 'add', 'recovery'], active: 'inspector' },
      ] },
      floats: [], closed: ['feedback', 'settings'], selected: 'source',
    });

    expect(migrated.version).toBe(4);
    expect(migrated.selected).toBe('source');
    // The operator closed feedback and settings under v3; migration must not undo that.
    expect(migrated.closed).toEqual(['feedback', 'settings']);
    expect(migrated.root.children[1]).toMatchObject({
      tabs: ['source', 'findings', 'model-preview'], active: 'source' });
    expect(migrated.root.children[2]).toMatchObject({
      tabs: ['inspector', 'add', 'recovery', 'models', 'sound'], active: 'inspector' });
  });

  it('refuses a panel a stored v3 layout could not have named', () => {
    const migrated = normalizeWorkshopLayout({
      version: 3, floats: [], closed: [], selected: 'source',
      root: { type: 'tabs', tabs: ['source', 'model-preview', 'sound'], active: 'model-preview' },
    });
    const placed = JSON.stringify(migrated.root);
    expect(placed).toContain('"source"');
    // v3 had no media vocabulary: these enter through migration, never out of the stored tree.
    expect(migrated.selected).toBe('source');
    expect(migrated.root.active).toBe('source');
  });

  it('classifies the source and model preview as document panels', () => {
    expect(workshopLayoutModel.kind('source')).toBe('document');
    expect(workshopLayoutModel.kind('model-preview')).toBe('document');
    for (const panel of ['files', 'inspector', 'models', 'sound', 'settings']) {
      expect(workshopLayoutModel.kind(panel)).toBe('tool');
    }
    expect(workshopLayoutModel.kind('nonexistent')).toBeNull();
  });

  it('preserves and renormalizes sibling proportions when removing a panel', () => {
    const state = defaultWorkshopLayout();
    state.root.sizes = [10, 30, 60];
    const closed = closeWorkshopPanel(state, 'source');
    expect(closed.root.sizes).toEqual([10, 30, 60]);
  });

  it('clamps loaded, floated and dragged windows to the current canvas', () => {
    const bounds = { width: 640, height: 480 };
    let state = floatWorkshopPanel(defaultWorkshopLayout(), 'inspector', {
      x: 900, y: -20, width: 900, height: 700,
    }, bounds);
    expect(state.floats[0]).toMatchObject({ x: 0, y: 0, width: 640, height: 480 });

    state = moveWorkshopFloat(state, 'inspector', 700, 600, bounds);
    expect(state.floats[0]).toMatchObject({ x: 0, y: 0, width: 640, height: 480 });

    const restored = normalizeWorkshopLayout({
      ...state,
      floats: [{ panel: 'inspector', x: 500, y: 400, width: 300, height: 200 }],
    }, bounds);
    expect(restored.floats[0]).toMatchObject({ x: 340, y: 280, width: 300, height: 200 });
  });
});
