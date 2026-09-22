import { describe, expect, it } from 'vitest';
import { defaultLiveLayout, liveLayoutModel, normalizeLiveLayout, restorePreviousLiveLayout } from '../../gui/live-layout-model.js';
import { prepareOperatorProfileImport, serializeOperatorProfile, createDefaultOperatorProfile } from '../../gui/operator-profile.js';
import { createSemanticActionRegistry } from '../../gui/semantic-action-registry.js';

const group = tabs => ({ type: 'tabs', tabs, active: tabs[0] });
const tabs = node => !node ? [] : node.type === 'tabs' ? node.tabs : node.children.flatMap(tabs);
describe('compact Live dock layout', () => {
  it('starts with exactly four panes, leaving detail tools and saves closed', () => {
    const layout = defaultLiveLayout();
    expect(layout.version).toBe(19);
    expect(tabs(layout.root)).toEqual(['roster', 'map', 'inspector', 'activity']);
    expect(layout.floats).toEqual([]);
    for (const panel of ['manual-save', 'comms', 'presentation', 'checkpoint', 'readiness', 'mission', 'station-console']) {
      expect(layout.closed).toContain(panel);
    }
    expect(liveLayoutModel.pinned).toEqual([]); // Critical warnings live in the header.
  });
  it.each(Array.from({ length: 18 }, (_, i) => i + 1))('resets generation %i exactly once', version => {
    const old = { version, root: group(['roster', 'station', 'join']), floats: [], closed: [], selected: 'station' };
    expect(normalizeLiveLayout(old)).toEqual(defaultLiveLayout());
    const changed = liveLayoutModel.float(defaultLiveLayout(), 'map', { x: 20, y: 30 });
    expect(normalizeLiveLayout(changed)).toEqual(changed);
  });
  it('backs up old arrangements and explicitly restores aliases without losing density', () => {
    const old = { version: 18, root: group(['roster', 'station', 'objective', 'journal', 'health']), floats: [], closed: [], selected: 'station' };
    const registry = createSemanticActionRegistry();
    const imported = prepareOperatorProfileImport(JSON.stringify({ ...createDefaultOperatorProfile(), liveLayout: old, gmDensity: 'touch' }), { registry });
    const profile = imported.profile;
    expect(profile.liveLayout).toEqual(defaultLiveLayout());
    expect(profile.gmDensity).toBe('touch');
    const restored = restorePreviousLiveLayout(profile.previousLiveLayout);
    expect(tabs(restored.root)).toEqual(['roster', 'station-console', 'mission', 'activity', 'readiness']);
    const again = prepareOperatorProfileImport(serializeOperatorProfile({ ...profile, liveLayout: restored }), { registry }).profile;
    expect(again.liveLayout).toEqual(restored);
    expect(again.previousLiveLayout).toEqual(profile.previousLiveLayout);
  });
  it('reorders one strip without changing active tabs or float geometry', () => {
    const normalized = normalizeLiveLayout({ version: 19, root: group(['roster', 'map', 'inspector']), floats: [], closed: [], selected: 'roster' });
    const reordered = liveLayoutModel.reorder(normalized, 'inspector', 'roster');
    expect(reordered.root.tabs).toEqual(['inspector', 'roster', 'map']);
    expect(reordered.root.active).toBe('roster');
    expect(reordered.selected).toBe('roster');
    expect(normalizeLiveLayout(reordered)).toEqual(reordered);
    expect(liveLayoutModel.reorder(reordered, 'inspector').root.tabs).toEqual(['roster', 'map', 'inspector']);
    expect(normalized.root.tabs).toEqual(['roster', 'map', 'inspector']);
  });
  it('uses shared docking and floating transitions', () => {
    const floated = liveLayoutModel.float(defaultLiveLayout(), 'map', { x: 20, y: 30 });
    expect(floated.floats).toEqual([{ panel: 'map', x: 20, y: 30, width: 420, height: 360 }]);
    const docked = liveLayoutModel.dock(floated, 'map', 'roster', 'tab');
    expect(docked.floats).toEqual([]);
    expect(liveLayoutModel.close(docked, 'map').closed).toContain('map');
  });
});
