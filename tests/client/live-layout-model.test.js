import { describe, expect, it } from 'vitest';
import { defaultLiveLayout, liveLayoutModel, normalizeLiveLayout } from '../../gui/live-layout-model.js';

describe('Live dock layout model', () => {
  it('keeps the four workflow panels in a versioned layout separate from Workshop', () => {
    expect(defaultLiveLayout()).toMatchObject({ version: 1, selected: 'roster' });
    expect(liveLayoutModel.panels).toEqual(['roster', 'readiness', 'join', 'manual-save']);
  });

  it('repairs obsolete, malformed and duplicate layouts', () => {
    expect(normalizeLiveLayout({ version: 99 })).toEqual(defaultLiveLayout());
    expect(normalizeLiveLayout({ version: 1, root: { type: 'tabs', tabs: ['roster', 'roster', 'unsafe'] },
      floats: [{ panel: 'join' }], closed: ['readiness'], selected: 'unsafe' })).toEqual({
      version: 1, root: { type: 'tabs', tabs: ['roster'], active: 'roster' },
      floats: [{ panel: 'join', x: 12, y: 12, width: 420, height: 360 }],
      closed: ['readiness', 'manual-save'], selected: 'roster',
    });
  });

  it('uses the same pointer and keyboard transition model as Workshop', () => {
    const moved = liveLayoutModel.dock(defaultLiveLayout(), 'join', 'roster', 'left');
    expect(JSON.stringify(moved.root)).toContain('"join"');
    expect(moved.selected).toBe('join');
    expect(liveLayoutModel.close(moved, 'join').closed).toContain('join');
  });
});
