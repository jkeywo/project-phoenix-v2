import { describe, expect, it } from 'vitest';
import { defaultLiveLayout, liveLayoutModel, normalizeLiveLayout } from '../../gui/live-layout-model.js';

describe('Live dock layout model', () => {
  it('registers the workflow and record panels in a versioned layout separate from Workshop', () => {
    expect(defaultLiveLayout()).toMatchObject({ version: 2, selected: 'roster' });
    expect(liveLayoutModel.panels).toEqual(['roster', 'readiness', 'join', 'manual-save',
      'mission', 'comms', 'activity', 'journal', 'session-history']);
    // Comms, the activity feed, the action journal and the session history keep
    // the one tab relationship the centre region gave them.
    expect(defaultLiveLayout().root.children[1]).toEqual({
      type: 'tabs', tabs: ['comms', 'activity', 'journal', 'session-history'], active: 'comms' });
  });

  it('repairs obsolete, malformed and duplicate layouts', () => {
    expect(normalizeLiveLayout({ version: 99 })).toEqual(defaultLiveLayout());
    expect(normalizeLiveLayout({ version: 2, root: { type: 'tabs', tabs: ['roster', 'roster', 'unsafe'] },
      floats: [{ panel: 'join' }], closed: ['readiness'], selected: 'unsafe' })).toEqual({
      version: 2, root: { type: 'tabs', tabs: ['roster'], active: 'roster' },
      floats: [{ panel: 'join', x: 12, y: 12, width: 420, height: 360 }],
      closed: ['readiness', 'manual-save', 'mission', 'comms', 'activity', 'journal', 'session-history'],
      selected: 'roster',
    });
  });

  it('registers the record panels on a stored v1 layout as their own tab group', () => {
    const migrated = normalizeLiveLayout({
      version: 1, root: { type: 'tabs', tabs: ['roster', 'readiness', 'join', 'manual-save'], active: 'roster' },
      floats: [], closed: [], selected: 'roster',
    });
    expect(migrated).toEqual(defaultLiveLayout());
  });

  it('leaves a panel a stored v1 layout closed closed, and refuses one it never registered', () => {
    const migrated = normalizeLiveLayout({
      version: 1, root: { type: 'tabs', tabs: ['roster', 'readiness'], active: 'readiness' },
      // v1 had no record vocabulary: `comms` here must not be read back out.
      floats: [{ panel: 'comms', x: 5, y: 6, width: 300, height: 200 }],
      closed: ['join', 'manual-save'], selected: 'readiness',
    });
    expect(migrated.version).toBe(2);
    expect(migrated.floats).toEqual([]);
    expect(migrated.closed).toEqual(['join', 'manual-save']);
    expect(migrated.root.children[0]).toMatchObject({ tabs: ['roster', 'readiness', 'mission'], active: 'readiness' });
    expect(migrated.root.children[1]).toEqual({
      type: 'tabs', tabs: ['comms', 'activity', 'journal', 'session-history'], active: 'comms' });
  });

  it('uses the same pointer and keyboard transition model as Workshop', () => {
    const moved = liveLayoutModel.dock(defaultLiveLayout(), 'join', 'roster', 'left');
    expect(JSON.stringify(moved.root)).toContain('"join"');
    expect(moved.selected).toBe('join');
    expect(liveLayoutModel.close(moved, 'join').closed).toContain('join');
    // A record panel separates from its default group like any other.
    const separated = liveLayoutModel.dock(defaultLiveLayout(), 'journal', 'roster', 'right');
    expect(separated.root.children[1]).toEqual({
      type: 'tabs', tabs: ['comms', 'activity', 'session-history'], active: 'comms' });
  });
});
