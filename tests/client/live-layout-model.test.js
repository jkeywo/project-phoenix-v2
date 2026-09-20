import { describe, expect, it } from 'vitest';
import { defaultLiveLayout, liveLayoutModel, normalizeLiveLayout } from '../../gui/live-layout-model.js';

describe('Live dock layout model', () => {
  it('registers the workflow and record panels in a versioned layout separate from Workshop', () => {
    expect(defaultLiveLayout()).toMatchObject({ version: 18, selected: 'roster' });
    // Every registered panel, in the order the narrow switcher offers them.
    expect(liveLayoutModel.panels).toEqual(['roster', 'readiness', 'join', 'manual-save',
      'mission', 'comms', 'activity', 'journal', 'session-history',
      'map', 'attention', 'workload', 'widgets', 'health', 'station', 'station-console',
      'presentation', 'audition', 'source-link', 'spawn', 'inspector', 'checkpoint', 'restore',
      'contact', 'npc', 'misclassify', 'report-policy', 'system', 'effect',
      'despawn', 'faction', 'objective', 'entity-fields', 'world-fields', 'hull-fields', 'region-fields', 'presentation-fields']);
    // Spawn is a DRAFT, not a place: absent from the default arrangement.
    expect(liveLayoutModel.temporary)
      .toEqual(['spawn', 'restore', 'misclassify', 'report-policy', 'effect']);
    expect(defaultLiveLayout().closed)
      .toEqual(['spawn', 'restore', 'misclassify', 'report-policy', 'effect']);
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
      // Every selected-entity tool reads the same selection the inspector does,
      // so they share its column as tabs (issues #1510, #1511 and #1512).
      { type: 'tabs',
        tabs: ['inspector', 'contact', 'npc', 'system', 'despawn', 'faction', 'entity-fields', 'hull-fields', 'region-fields', 'presentation-fields'],
        active: 'inspector' },
    ] });
    // The operator's own instruments open a group of their own.
    expect(defaultLiveLayout().root.children[0].children[0].children[1]).toEqual({ type: 'tabs', tabs: ['presentation', 'audition', 'source-link'], active: 'presentation' });
  });

  it('repairs obsolete, malformed and duplicate layouts', () => {
    expect(normalizeLiveLayout({ version: 99 })).toEqual(defaultLiveLayout());
    expect(normalizeLiveLayout({ version: 14, root: { type: 'tabs', tabs: ['roster', 'roster', 'unsafe'] },
      floats: [{ panel: 'join' }], closed: ['readiness'], selected: 'unsafe' })).toEqual({
      version: 18, root: { type: 'tabs', tabs: ['roster', 'world-fields', 'hull-fields', 'region-fields', 'presentation-fields', 'attention', 'health'], active: 'roster' },
      floats: [{ panel: 'join', x: 12, y: 12, width: 420, height: 360 }],
      closed: ['readiness', 'manual-save', 'mission', 'comms', 'activity', 'journal', 'session-history',
        'map', 'workload', 'widgets', 'station', 'station-console',
        'presentation', 'audition', 'source-link', 'spawn', 'inspector', 'checkpoint', 'restore',
        'contact', 'npc', 'misclassify', 'report-policy', 'system', 'effect',
        'despawn', 'faction', 'objective', 'entity-fields'],
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
    expect(migrated.version).toBe(18);
    expect(migrated.floats).toEqual([]);
    // A draft nobody opened is closed, which is what "not open" means for one.
    expect(migrated.closed).toEqual(['join', 'manual-save', 'spawn', 'restore',
      'misclassify', 'report-policy', 'effect']);
    expect(migrated.root.children[0].children[0].children[0]).toMatchObject({
      tabs: ['roster', 'readiness', 'mission', 'attention', 'workload', 'widgets', 'station',
        'objective', 'world-fields'],
      active: 'readiness' });
    expect(migrated.root.children[0].children[0].children[1]).toEqual({ type: 'tabs', tabs: ['presentation', 'audition', 'source-link'], active: 'presentation' });
    expect(migrated.root.children[0].children[1]).toEqual({ type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
      { type: 'tabs', tabs: ['map', 'station-console'], active: 'map' },
      { type: 'tabs',
        tabs: ['inspector', 'contact', 'npc', 'system', 'despawn', 'faction', 'entity-fields', 'hull-fields', 'region-fields', 'presentation-fields'],
        active: 'inspector' },
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
      version: 18,
      root: { type: 'split', axis: 'vertical', sizes: [1, 1], children: [
        { type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
          { type: 'split', axis: 'vertical', sizes: [1, 1], children: [
            { type: 'tabs',
              tabs: ['roster', 'mission', 'attention', 'workload', 'widgets', 'station',
                'objective', 'world-fields'],
              active: 'roster' },
            { type: 'tabs', tabs: ['presentation', 'audition', 'source-link'], active: 'presentation' },
          ] },
          { type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
            { type: 'tabs', tabs: ['map', 'station-console'], active: 'map' },
            { type: 'tabs',
              tabs: ['inspector', 'contact', 'npc', 'system', 'despawn', 'faction',
                'entity-fields', 'hull-fields', 'region-fields', 'presentation-fields'],
              active: 'inspector' },
          ] },
        ] },
        { type: 'tabs', tabs: ['comms', 'journal', 'health', 'checkpoint'], active: 'journal' },
      ] },
      floats: [],
      // Panels the operator closed under v2 stay closed, and an unopened draft
      // joins them.
      closed: ['readiness', 'join', 'manual-save', 'activity', 'session-history', 'spawn', 'restore',
        'misclassify', 'report-policy', 'effect'],
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

    expect(migrated.version).toBe(18);
    // The console joins the map; its controls join the workflow group; the
    // inspector takes a column of its own beside them.
    expect(migrated.root.children[1]).toEqual({
      type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
        { type: 'tabs', tabs: ['map', 'station-console'], active: 'map' },
        { type: 'tabs',
        tabs: ['inspector', 'contact', 'npc', 'system', 'despawn', 'faction', 'entity-fields', 'hull-fields', 'region-fields', 'presentation-fields'],
        active: 'inspector' },
      ] });
    // `health` was closed and is pinned, so it is repaired back — after the
    // panels this migration registers, which is the order the native profile
    // sanitizer runs in too.
    expect(migrated.root.children[0].children[0].tabs)
      // `checkpoint` prefers the journal's group, which this tree closed, so it
      // falls back to the first visible panel like any other migrated panel.
      .toEqual(['roster', 'attention', 'station', 'checkpoint', 'objective', 'world-fields', 'health']);
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

    expect(migrated.version).toBe(18);
    // They open a group of their own under the panels they were beside.
    expect(migrated.root.children[1]).toEqual({ type: 'tabs', tabs: ['presentation', 'audition', 'source-link'], active: 'presentation' });
    // v4 had no utility vocabulary, so a stored tree cannot name one.
    expect(normalizeLiveLayout({
      version: 4, root: { type: 'tabs', tabs: ['roster', 'audition'], active: 'audition' },
      floats: [{ panel: 'source-link', x: 4, y: 5, width: 300, height: 200 }],
      closed: [], selected: 'audition',
    }).floats).toEqual([]);
  });

  it('registers System control on a stored v9 layout beside the tools it joins', () => {
    // v9 is the version every profile written before issue #1511 carries, so
    // this is the migration that actually runs — and the one the native
    // profile sanitizer has to agree with exactly.
    const migrated = normalizeLiveLayout({
      version: 9,
      root: { type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
        { type: 'tabs', tabs: ['roster', 'attention'], active: 'roster' },
        { type: 'tabs', tabs: ['inspector', 'contact'], active: 'contact' },
      ] },
      floats: [],
      closed: ['readiness', 'join', 'manual-save', 'mission', 'comms', 'activity', 'journal',
        'session-history', 'map', 'workload', 'widgets', 'health', 'station', 'station-console',
        'presentation', 'audition', 'source-link', 'spawn', 'checkpoint', 'restore', 'npc'],
      selected: 'contact',
    });
    expect(migrated.version).toBe(18);
    // It joins the group the inspector is in, keeping the operator's own tab —
    // and so do the panels the version after it registered (issue #1512).
    expect(migrated.root.children[1]).toMatchObject({
      tabs: ['inspector', 'contact', 'system', 'despawn', 'faction', 'entity-fields', 'hull-fields', 'region-fields', 'presentation-fields'],
      active: 'contact' });
    // A draft is never PLACED by migration: not open is what closed means.
    expect(migrated.closed).toContain('effect');
    expect(JSON.stringify(migrated.root)).not.toContain('"effect"');
    // v9 had no direct-effect vocabulary, so a stored tree cannot name one.
    expect(normalizeLiveLayout({
      version: 9, root: { type: 'tabs', tabs: ['roster', 'effect'], active: 'effect' },
      floats: [{ panel: 'system', x: 4, y: 5, width: 300, height: 200 }],
      closed: [], selected: 'effect',
    }).floats).toEqual([]);
  });

  it('retires the ghost draft from a stored v13 layout wherever it held it', () => {
    // Version 14 registered nothing and RETIRED a panel: placing a ghost is a
    // Spawn outcome now. A version-13 profile could hold the draft docked, as
    // the selected panel, or floating; none of those may come back.
    const docked = normalizeLiveLayout({
      version: 13, root: { type: 'tabs', tabs: ['roster', 'contact', 'ghost'], active: 'ghost' },
      floats: [], closed: liveLayoutModel.panels.filter(panel => !['roster', 'contact'].includes(panel)),
      selected: 'ghost',
    });
    expect(docked.version).toBe(18);
    expect(JSON.stringify(docked)).not.toContain('"ghost"');
    expect(docked.root.tabs.slice(0, 2)).toEqual(['roster', 'contact']);
    // The view falls to what is left rather than to a panel that is gone.
    expect(docked.root.active).toBe('roster');
    expect(docked.selected).toBe('roster');
    const floating = normalizeLiveLayout({
      version: 13,
      root: { type: 'split', axis: 'horizontal', sizes: [2, 3], children: [
        { type: 'tabs', tabs: ['ghost'], active: 'ghost' }, { type: 'tabs', tabs: ['roster', 'map'], active: 'map' },
      ] },
      floats: [{ panel: 'ghost', x: 8, y: 9, width: 300, height: 200 }],
      closed: liveLayoutModel.panels.filter(panel => !['roster', 'map'].includes(panel)),
      selected: 'ghost',
    });
    expect(JSON.stringify(floating)).not.toContain('"ghost"');
    expect(floating.floats).toEqual([]);
    // A group left empty goes, and a split left with one child collapses.
    expect(floating.root.type).toBe('tabs');
    expect(floating.root.tabs.slice(0, 2)).toEqual(['roster', 'map']);
    // And the current vocabulary cannot name it either.
    expect(normalizeLiveLayout({ version: 14, root: { type: 'tabs', tabs: ['roster', 'ghost'], active: 'ghost' },
      floats: [], closed: [], selected: 'ghost' }).root.tabs).not.toContain('ghost');
  });

  it('refuses to leave the attention and health panels closed', () => {
    // The attention region renders connection and recovery banners verbatim and
    // health is the table behind them. A Game Master must not be able to hide a
    // failure from themselves, so neither closes — by control or by profile.
    expect(liveLayoutModel.pinned).toEqual(['attention', 'health']);
    const closedEverything = normalizeLiveLayout({
      version: 14, root: { type: 'tabs', tabs: ['roster'], active: 'roster' }, floats: [],
      closed: liveLayoutModel.panels.filter(panel => panel !== 'roster'), selected: 'roster',
    });
    expect(closedEverything.closed).not.toContain('attention');
    expect(closedEverything.closed).not.toContain('health');
    expect(closedEverything.root.tabs).toEqual(['roster', 'world-fields', 'hull-fields', 'region-fields', 'presentation-fields', 'attention', 'health']);
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
