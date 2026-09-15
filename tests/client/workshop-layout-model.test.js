import { describe, expect, it } from 'vitest';
import {
  closeWorkshopPanel, defaultWorkshopLayout, dockWorkshopPanel, floatWorkshopPanel,
  moveWorkshopFloat, normalizeWorkshopLayout, reopenWorkshopPanel,
} from '../../gui/workshop-layout-model.js';

const panels = state => JSON.stringify(state.root) + JSON.stringify(state.floats);

describe('Workshop layout model', () => {
  it('tabs, splits, floats, moves, closes, reopens and resets all panels', () => {
    let state = defaultWorkshopLayout();
    state = dockWorkshopPanel(state, 'inspector', 'source', 'tab');
    expect(state.root.children[1]).toMatchObject({ tabs: ['source', 'inspector'], active: 'inspector' });
    state = dockWorkshopPanel(state, 'files', 'source', 'bottom');
    expect(state.root).toMatchObject({ type: 'split', axis: 'vertical' });
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
  });

  it('repairs malformed, duplicate and obsolete layouts without losing panels', () => {
    const repaired = normalizeWorkshopLayout({ version: 1, selected: 'future', closed: ['future'], floats: [], root: {
      type: 'split', axis: 'sideways', children: [{ type: 'tabs', tabs: ['source', 'source', 'future'] }],
    } });
    expect(repaired).toEqual(defaultWorkshopLayout());
    expect(normalizeWorkshopLayout({ version: 99 })).toEqual(defaultWorkshopLayout());
  });

  it('deduplicates globally before applying collection limits', () => {
    const repaired = normalizeWorkshopLayout({
      version: 1,
      root: null,
      floats: [
        { panel: 'source' }, { panel: 'source' }, { panel: 'source' },
        { panel: 'inspector', x: 30, y: 40 }, { panel: 'files' },
      ],
      closed: ['source', 'inspector', 'files'], selected: 'inspector',
    });
    expect(repaired).toEqual({
      version: 1,
      root: null,
      floats: [
        { panel: 'source', x: 12, y: 12, width: 420, height: 360 },
        { panel: 'inspector', x: 30, y: 40, width: 420, height: 360 },
        { panel: 'files', x: 12, y: 12, width: 420, height: 360 },
      ],
      closed: [], selected: 'inspector',
    });
  });

  it('reopens into a new dock without disturbing existing floats', () => {
    let state = defaultWorkshopLayout();
    state = floatWorkshopPanel(state, 'files', { x: 18, y: 24, width: 300, height: 200 });
    state = floatWorkshopPanel(state, 'source', { x: 48, y: 54, width: 320, height: 220 });
    state = closeWorkshopPanel(state, 'inspector');
    const floats = state.floats;

    state = reopenWorkshopPanel(state, 'inspector');

    expect(state.root).toEqual({ type: 'tabs', tabs: ['inspector'], active: 'inspector' });
    expect(state.floats).toEqual(floats);
  });

  it('stagger-defaults newly floated panels and preserves explicit placement', () => {
    let state = floatWorkshopPanel(defaultWorkshopLayout(), 'files');
    state = floatWorkshopPanel(state, 'source');
    state = floatWorkshopPanel(state, 'inspector', { x: 7, y: 9 });
    expect(state.floats.map(({ x, y }) => [x, y])).toEqual([[24, 24], [52, 52], [7, 9]]);
  });

  it('defaults missing and malformed roots but preserves an explicit null root', () => {
    const closed = ['files', 'source', 'inspector'];
    expect(normalizeWorkshopLayout({ version: 1, floats: [], closed }))
      .toEqual(defaultWorkshopLayout());
    expect(normalizeWorkshopLayout({
      version: 1, root: { type: 'unknown' }, floats: [], closed,
    })).toEqual(defaultWorkshopLayout());
    expect(normalizeWorkshopLayout({
      version: 1, root: null, floats: [], closed, selected: 'source',
    })).toEqual({ version: 1, root: null, floats: [], closed, selected: 'files' });
  });

  it('defaults an untrusted layout whose split nesting exceeds the panel bound', () => {
    let root = { type: 'tabs', tabs: ['source'], active: 'source' };
    for (let depth = 0; depth < 100; depth += 1) {
      root = { type: 'split', axis: 'horizontal', sizes: [1], children: [root] };
    }
    expect(normalizeWorkshopLayout({
      version: 1, root, floats: [], closed: ['files', 'inspector'], selected: 'source',
    })).toEqual(defaultWorkshopLayout());
  });

  it('preserves and renormalizes sibling proportions when removing a panel', () => {
    const state = defaultWorkshopLayout();
    state.root.sizes = [10, 30, 60];
    const closed = closeWorkshopPanel(state, 'source');
    expect(closed.root.sizes).toEqual([100 / 7, 600 / 7]);
    expect(closed.root.sizes[1] / closed.root.sizes[0]).toBeCloseTo(6);
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
