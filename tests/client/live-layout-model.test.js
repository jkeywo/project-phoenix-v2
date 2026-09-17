import { describe, expect, it } from 'vitest';
import { defaultLiveLayout, liveLayoutModel, normalizeLiveLayout } from '../../gui/live-layout-model.js';

describe('Live dock layout model', () => {
  it('registers the workflow and record panels in a versioned layout separate from Workshop', () => {
    expect(defaultLiveLayout()).toMatchObject({ version: 8, selected: 'roster' });
    // Every registered panel, in the order the narrow switcher offers them.
    expect(liveLayoutModel.panels).toEqual(['roster', 'readiness', 'join', 'manual-save',
      'mission', 'comms', 'activity', 'journal', 'session-history',
      'map', 'attention', 'workload', 'widgets', 'health', 'station', 'station-console',
      'presentation', 'audition', 'source-link', 'spawn', 'inspector', 'checkpoint', 'restore']);
    // Spawn is a DRAFT, not a place: absent from the default arrangement.
    expect(liveLayoutModel.temporary).toEqual(['spawn', 'restore']);
    expect(defaultLiveLayout().closed).toEqual(['spawn', 'restore']);
    // The map and the authentic Station console are the surfaces this desk is
    // arranged around; they share a group by default.
    expect(liveLayoutModel.kind('map')).toBe('document');
    expect(liveLayoutModel.kind('station-console')).toBe('document');
    expect(liveLayoutModel.kind('station')).toBe('tool');
    expect(liveLayoutModel.kind('roster')).toBe('tool');
    // Comms, the activity feed, the action journal, the session history and
    // peer health keep the one tab relationship the centre region gave them.
    expect(defaultLiveLayout().root.children[1]).toEqual({
      type: 'tabs', tabs: ['comms', 'activity', 'journal', 'session-history', 'health', 'checkpoint'], active: 'comms' });
    // The map and the console share one column; the inspector has its own,
    // because it holds every selected-entity control.
    expect(defaultLiveLayout().root.children[0].children[1]).toEqual({ type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
      { type: 'tabs', tabs: ['map', 'station-console'], active: 'map' },
      { type: 'tabs', tabs: ['inspector'], active: 'inspector' },
    ] });
    // The operator's own instruments open a group of their own.
    expect(defaultLiveLayout().root.children[0].children[0].children[1]).toEqual({ type: 'tabs', tabs: ['presentation', 'audition', 'source-link'], active: 'presentation' });
  });

  it('repairs obsolete, malformed and duplicate layouts', () => {
    expect(normalizeLiveLayout({ version: 99 })).toEqual(defaultLiveLayout());
    expect(normalizeLiveLayout({ version: 8, root: { type: 'tabs', tabs: ['roster', 'roster', 'unsafe'] },
      floats: [{ panel: 'join' }], closed: ['readiness'], selected: 'unsafe' })).toEqual({
      version: 8, root: { type: 'tabs', tabs: ['roster', 'attention', 'health'], active: 'roster' },
      floats: [{ panel: 'join', x: 12, y: 12, width: 420, height: 360 }],
      closed: ['readiness', 'manual-save', 'mission', 'comms', 'activity', 'journal', 'session-history',
        'map', 'workload', 'widgets', 'station', 'station-console',
        'presentation', 'audition', 'source-link', 'spawn', 'inspector', 'checkpoint', 'restore'],
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
    expect(migrated.version).toBe(8);
    expect(migrated.floats).toEqual([]);
    // A draft nobody opened is closed, which is what "not open" means for one.
    expect(migrated.closed).toEqual(['join', 'manual-save', 'spawn', 'restore']);
    expect(migrated.root.children[0].children[0].children[0]).toMatchObject({
      tabs: ['roster', 'readiness', 'mission', 'attention', 'workload', 'widgets', 'station'],
      active: 'readiness' });
    expect(migrated.root.children[0].children[0].children[1]).toEqual({ type: 'tabs', tabs: ['presentation', 'audition', 'source-link'], active: 'presentation' });
    expect(migrated.root.children[0].children[1]).toEqual({ type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
      { type: 'tabs', tabs: ['map', 'station-console'], active: 'map' },
      { type: 'tabs', tabs: ['inspector'], active: 'inspector' },
    ] });
    expect(migrated.root.children[1]).toEqual({
      type: 'tabs', tabs: ['comms', 'activity', 'journal', 'session-history', 'health', 'checkpoint'], active: 'comms' });
  });

  it('registers the map and awareness panels on a stored v2 layout', () => {
    // v2 is the version every existing operator profile carries, so this is the
    // migration that actually runs — and the one the native profile sanitizer
    // has to agree with exactly.
    const migrated = normalizeLiveLayout({
      version: 2,
      root: { type: 'split', axis: 'vertical', sizes: [1, 1], children: [
        { type: 'tabs', tabs: ['roster', 'mission'], active: 'roster' },
        { type: 'tabs', tabs: ['comms', 'journal'], active: 'journal' },
      ] },
      floats: [],
      closed: ['readiness', 'join', 'manual-save', 'activity', 'session-history'],
      selected: 'journal',
    });

    expect(migrated).toEqual({
      version: 8,
      root: { type: 'split', axis: 'vertical', sizes: [1, 1], children: [
        { type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
          { type: 'split', axis: 'vertical', sizes: [1, 1], children: [
            { type: 'tabs', tabs: ['roster', 'mission', 'attention', 'workload', 'widgets', 'station'],
              active: 'roster' },
            { type: 'tabs', tabs: ['presentation', 'audition', 'source-link'], active: 'presentation' },
          ] },
          { type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
      { type: 'tabs', tabs: ['map', 'station-console'], active: 'map' },
      { type: 'tabs', tabs: ['inspector'], active: 'inspector' },
    ] },
        ] },
        { type: 'tabs', tabs: ['comms', 'journal', 'health', 'checkpoint'], active: 'journal' },
      ] },
      floats: [],
      // Panels the operator closed under v2 stay closed, and an unopened draft
      // joins them.
      closed: ['readiness', 'join', 'manual-save', 'activity', 'session-history', 'spawn', 'restore'],
      selected: 'journal',
    });
  });

  it('registers the Station panels on a stored v3 layout', () => {
    const migrated = normalizeLiveLayout({
      version: 3,
      root: { type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
        { type: 'tabs', tabs: ['roster', 'attention'], active: 'roster' },
        { type: 'tabs', tabs: ['map'], active: 'map' },
      ] },
      floats: [],
      closed: ['readiness', 'join', 'manual-save', 'mission', 'comms', 'activity', 'journal',
        'session-history', 'workload', 'widgets', 'health'],
      selected: 'roster',
    });

    expect(migrated.version).toBe(8);
    // The console joins the map; its controls join the workflow group; the
    // inspector takes a column of its own beside them.
    expect(migrated.root.children[1]).toEqual({
      type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
        { type: 'tabs', tabs: ['map', 'station-console'], active: 'map' },
        { type: 'tabs', tabs: ['inspector'], active: 'inspector' },
      ] });
    // `health` was closed and is pinned, so it is repaired back — after the
    // panels this migration registers, which is the order the native profile
    // sanitizer runs in too.
    expect(migrated.root.children[0].children[0].tabs)
      // `checkpoint` prefers the journal's group, which this tree closed, so it
      // falls back to the first visible panel like any other migrated panel.
      .toEqual(['roster', 'attention', 'station', 'checkpoint', 'health']);
    expect(migrated.root.children[0].children[1]).toEqual({ type: 'tabs', tabs: ['presentation', 'audition', 'source-link'], active: 'presentation' });
    // v3 had no Station vocabulary, so a stored tree cannot name one.
    expect(normalizeLiveLayout({
      version: 3, root: { type: 'tabs', tabs: ['roster', 'station-console'], active: 'station-console' },
      floats: [{ panel: 'station', x: 4, y: 5, width: 300, height: 200 }],
      closed: [], selected: 'station-console',
    }).floats).toEqual([]);
  });

  it('registers the operator utility panels on a stored v4 layout', () => {
    const migrated = normalizeLiveLayout({
      version: 4,
      root: { type: 'tabs', tabs: ['roster', 'mission'], active: 'mission' },
      floats: [],
      closed: ['readiness', 'join', 'manual-save', 'comms', 'activity', 'journal', 'session-history',
        'map', 'workload', 'widgets', 'station', 'station-console'],
      selected: 'mission',
    });

    expect(migrated.version).toBe(8);
    // They open a group of their own under the panels they were beside.
    expect(migrated.root.children[1]).toEqual({ type: 'tabs', tabs: ['presentation', 'audition', 'source-link'], active: 'presentation' });
    // v4 had no utility vocabulary, so a stored tree cannot name one.
    expect(normalizeLiveLayout({
      version: 4, root: { type: 'tabs', tabs: ['roster', 'audition'], active: 'audition' },
      floats: [{ panel: 'source-link', x: 4, y: 5, width: 300, height: 200 }],
      closed: [], selected: 'audition',
    }).floats).toEqual([]);
  });

  it('refuses to leave the attention and health panels closed', () => {
    // The attention region renders connection and recovery banners verbatim and
    // health is the table behind them. A Game Master must not be able to hide a
    // failure from themselves, so neither closes — by control or by profile.
    expect(liveLayoutModel.pinned).toEqual(['attention', 'health']);
    const closedEverything = normalizeLiveLayout({
      version: 8, root: { type: 'tabs', tabs: ['roster'], active: 'roster' }, floats: [],
      closed: liveLayoutModel.panels.filter(panel => panel !== 'roster'), selected: 'roster',
    });
    expect(closedEverything.closed).not.toContain('attention');
    expect(closedEverything.closed).not.toContain('health');
    expect(closedEverything.root.tabs).toEqual(['roster', 'attention', 'health']);
    // And the model refuses the transition outright.
    const attempted = liveLayoutModel.close(defaultLiveLayout(), 'attention');
    expect(attempted).toEqual(defaultLiveLayout());
    expect(liveLayoutModel.close(defaultLiveLayout(), 'journal').closed).toContain('journal');
    // Pinned is not frozen: they may still be moved.
    expect(liveLayoutModel.float(defaultLiveLayout(), 'health', { x: 5, y: 6 }).floats)
      .toEqual([{ panel: 'health', x: 5, y: 6, width: 420, height: 360 }]);
  });

  it('uses the same pointer and keyboard transition model as Workshop', () => {
    const moved = liveLayoutModel.dock(defaultLiveLayout(), 'join', 'roster', 'left');
    expect(JSON.stringify(moved.root)).toContain('"join"');
    expect(moved.selected).toBe('join');
    expect(liveLayoutModel.close(moved, 'join').closed).toContain('join');
    // A record panel separates from its default group like any other.
    const separated = liveLayoutModel.dock(defaultLiveLayout(), 'journal', 'roster', 'right');
    expect(separated.root.children[1]).toEqual({
      type: 'tabs', tabs: ['comms', 'activity', 'session-history', 'health', 'checkpoint'],
      active: 'comms' });
    // And so does the map document, without disturbing the rest.
    const floated = liveLayoutModel.float(defaultLiveLayout(), 'map', { x: 20, y: 30 });
    expect(floated.floats).toEqual([{ panel: 'map', x: 20, y: 30, width: 420, height: 360 }]);
    expect(JSON.stringify(floated.root)).not.toContain('"map"');
  });
});
