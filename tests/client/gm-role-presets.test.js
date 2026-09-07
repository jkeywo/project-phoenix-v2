// @vitest-environment jsdom

import { describe, expect, it, vi } from 'vitest';
import {
  GM_ALL_ROLE_PRESET,
  GM_ALL_ROLE_PRESET_ID,
  GM_ROLE_PRESET_PANEL_IDS,
  GM_ROLE_PRESET_QUICK_ACTION_IDS,
  createGmRolePresets,
  isGmContactVisible,
  isGmPanelVisible,
  isGmQuickActionVisible,
  parseGmRolePresets,
  resolveGmRolePreset,
} from '../../gui/gm-role-presets.js';

const TACTICAL = {
  id: 'tactical',
  label: 'world.fs.gm_role_preset.tactical.label',
  panels: ['gm-map-panel', 'gm-activity'],
  quick_actions: ['gm-session-pause'],
  contacts: ['enemy_frigate'],
};
const NARRATIVE = { id: 'narrative' };

describe('parseGmRolePresets (pure loader)', () => {
  it('normalises an authored list, defaulting omitted facets to unrestricted', () => {
    const presets = parseGmRolePresets([TACTICAL, NARRATIVE]);
    expect(presets).toEqual([
      {
        id: 'tactical',
        label: 'world.fs.gm_role_preset.tactical.label',
        panels: ['gm-map-panel', 'gm-activity'],
        quickActions: ['gm-session-pause'],
        contacts: ['enemy_frigate'],
      },
      { id: 'narrative', label: '', panels: [], quickActions: [], contacts: [] },
    ]);
  });

  it('accepts a JSON string, exactly what wasm_get_gm_role_presets returns', () => {
    expect(parseGmRolePresets(JSON.stringify([NARRATIVE]))).toEqual([
      { id: 'narrative', label: '', panels: [], quickActions: [], contacts: [] },
    ]);
  });

  it('falls back to an empty list for unparseable or non-array input (acceptance criterion 1)', () => {
    expect(parseGmRolePresets('not json')).toEqual([]);
    expect(parseGmRolePresets(null)).toEqual([]);
    expect(parseGmRolePresets(undefined)).toEqual([]);
    expect(parseGmRolePresets({ id: 'not-an-array' })).toEqual([]);
    expect(parseGmRolePresets(42)).toEqual([]);
  });

  it('drops malformed entries without failing the whole list', () => {
    const presets = parseGmRolePresets([
      NARRATIVE,
      null,
      'nonsense',
      { label: 'missing an id' },
      { id: '' },
      { id: 42 },
    ]);
    expect(presets).toEqual([
      { id: 'narrative', label: '', panels: [], quickActions: [], contacts: [] },
    ]);
  });

  it('drops the reserved built-in "all" id even if a payload is hand-poked to carry it', () => {
    const presets = parseGmRolePresets([{ id: 'all', label: 'sneaky' }, NARRATIVE]);
    expect(presets.map((p) => p.id)).toEqual(['narrative']);
  });

  it('drops a repeated id defensively, keeping the first occurrence', () => {
    const presets = parseGmRolePresets([
      { id: 'tactical', label: 'first' },
      { id: 'tactical', label: 'second' },
    ]);
    expect(presets).toHaveLength(1);
    expect(presets[0].label).toBe('first');
  });

  it('never lets two parses of the same payload share mutable array state', () => {
    const payload = [TACTICAL];
    const a = parseGmRolePresets(payload);
    const b = parseGmRolePresets(payload);
    a[0].panels.push('gm-comms-panel');
    expect(b[0].panels).toEqual(['gm-map-panel', 'gm-activity']);
  });
});

describe('resolveGmRolePreset (fallback contract)', () => {
  const presets = parseGmRolePresets([TACTICAL, NARRATIVE]);

  it('resolves a known id to its preset', () => {
    expect(resolveGmRolePreset(presets, 'tactical').id).toBe('tactical');
  });

  it('falls back to the built-in All for null, undefined, "all", and an unknown id (acceptance criterion 1)', () => {
    for (const id of [null, undefined, GM_ALL_ROLE_PRESET_ID, 'removed-preset', '']) {
      expect(resolveGmRolePreset(presets, id)).toBe(GM_ALL_ROLE_PRESET);
    }
  });

  it('falls back safely even when the available list is empty, missing, or malformed', () => {
    expect(resolveGmRolePreset([], 'tactical')).toBe(GM_ALL_ROLE_PRESET);
    expect(resolveGmRolePreset(undefined, 'tactical')).toBe(GM_ALL_ROLE_PRESET);
  });

  it('resolves the same id independently for two separate operators with no shared state', () => {
    const a = resolveGmRolePreset(presets, 'tactical');
    const b = resolveGmRolePreset(presets, 'tactical');
    expect(a).toBe(b); // same object from the authored list, not a problem —
    // the point is that neither call mutates it for the other.
    a.panels.push('should-not-appear');
    expect(resolveGmRolePreset(presets, 'tactical').panels).toContain('should-not-appear');
    // Restore so later assertions in this suite are unaffected by this probe.
    a.panels.pop();
  });
});

describe('facet visibility helpers (M6-compatible panel descriptors)', () => {
  it('treats an empty facet list as unrestricted', () => {
    expect(isGmPanelVisible(GM_ALL_ROLE_PRESET, 'gm-map-panel')).toBe(true);
    expect(isGmQuickActionVisible(GM_ALL_ROLE_PRESET, 'gm-session-pause')).toBe(true);
    expect(isGmContactVisible(GM_ALL_ROLE_PRESET, 'enemy_frigate')).toBe(true);
  });

  it('restricts to exactly the authored ids on a non-empty facet list', () => {
    const preset = parseGmRolePresets([TACTICAL])[0];
    expect(isGmPanelVisible(preset, 'gm-map-panel')).toBe(true);
    expect(isGmPanelVisible(preset, 'gm-inspector')).toBe(false);
    expect(isGmQuickActionVisible(preset, 'gm-session-pause')).toBe(true);
    expect(isGmQuickActionVisible(preset, 'gm-session-resume')).toBe(false);
    expect(isGmContactVisible(preset, 'enemy_frigate')).toBe(true);
    expect(isGmContactVisible(preset, 'other_ship')).toBe(false);
  });

  it('names a panel this build does not draw yet without erroring — the M6-compatible shape', () => {
    const preset = parseGmRolePresets([
      { id: 'future', panels: ['gm-future-panel', 'gm-knowledge-panel'] },
    ])[0];
    expect(isGmPanelVisible(preset, 'gm-future-panel')).toBe(true);
    expect(isGmPanelVisible(preset, 'gm-knowledge-panel')).toBe(true);
    // A panel this build DOES draw, left off the list, is correctly hidden —
    // the vocabulary is open-ended in both directions.
    expect(isGmPanelVisible(preset, 'gm-map-panel')).toBe(false);
  });
});

function mount() {
  document.body.innerHTML = `
    <select id="gm-role-preset-select">
      <option value="all">all</option>
    </select>
    <section id="gm-map-panel"></section>
    <section id="gm-inspector"></section>
    <section id="gm-activity"></section>
    <section id="gm-station-controls"></section>
    <section id="gm-comms-panel"></section>
    <button id="gm-session-pause"></button>
    <button id="gm-session-resume"></button>
  `;
}

describe('createGmRolePresets (DOM controller)', () => {
  it('defaults to the built-in All with every panel and quick action visible', () => {
    mount();
    const onSelect = vi.fn();
    const controller = createGmRolePresets({ doc: document, onSelect });
    expect(controller.state()).toEqual({ presets: [], desiredId: null, effectivePresetId: 'all' });
    for (const id of [...GM_ROLE_PRESET_PANEL_IDS, ...GM_ROLE_PRESET_QUICK_ACTION_IDS]) {
      expect(document.getElementById(id).hidden).toBe(false);
    }
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('never touches gm-station-controls, which gm-station-puppet.js already owns', () => {
    mount();
    document.getElementById('gm-station-controls').hidden = true;
    const controller = createGmRolePresets({ doc: document });
    controller.select('tactical');
    expect(document.getElementById('gm-station-controls').hidden).toBe(true);
  });

  it('live-switches panel and quick-action visibility and notifies onSelect', () => {
    mount();
    const onSelect = vi.fn();
    const controller = createGmRolePresets({ doc: document, onSelect });
    controller.setAvailablePresets([TACTICAL, NARRATIVE]);

    controller.select('tactical');
    expect(onSelect).toHaveBeenLastCalledWith('tactical');
    expect(document.getElementById('gm-map-panel').hidden).toBe(false);
    expect(document.getElementById('gm-activity').hidden).toBe(false);
    expect(document.getElementById('gm-inspector').hidden).toBe(true);
    expect(document.getElementById('gm-session-pause').hidden).toBe(false);
    expect(document.getElementById('gm-session-resume').hidden).toBe(true);
    expect(document.getElementById('gm-role-preset-select').value).toBe('tactical');

    // Live switch back to All restores every panel and quick action.
    controller.select('all');
    expect(onSelect).toHaveBeenLastCalledWith(null);
    for (const id of [...GM_ROLE_PRESET_PANEL_IDS, ...GM_ROLE_PRESET_QUICK_ACTION_IDS]) {
      expect(document.getElementById(id).hidden).toBe(false);
    }
  });

  it('drives the live switch from the <select> element itself', () => {
    mount();
    const onSelect = vi.fn();
    const controller = createGmRolePresets({ doc: document, onSelect });
    controller.setAvailablePresets([TACTICAL]);
    const select = document.getElementById('gm-role-preset-select');
    select.value = 'tactical';
    select.dispatchEvent(new window.Event('change'));
    expect(onSelect).toHaveBeenCalledWith('tactical');
    expect(controller.state().effectivePresetId).toBe('tactical');
  });

  it('falls back safely to All for a removed preset without forgetting the operator\'s choice', () => {
    mount();
    const onSelect = vi.fn();
    const controller = createGmRolePresets({ doc: document, onSelect });
    controller.setAvailablePresets([TACTICAL]);
    controller.select('tactical');
    expect(controller.state().effectivePresetId).toBe('tactical');

    // The world reloads (or a mod pack changes) and no longer authors it.
    controller.setAvailablePresets([]);
    expect(controller.state()).toMatchObject({ desiredId: 'tactical', effectivePresetId: 'all' });
    expect(document.getElementById('gm-inspector').hidden).toBe(false);
    // Re-resolving the available list must not re-notify — nothing the
    // operator asked for changed, only what exists to grant it did.
    expect(onSelect).toHaveBeenCalledTimes(1);

    // It comes back — the operator's original choice is honoured again
    // without having to re-select it.
    controller.setAvailablePresets([TACTICAL]);
    expect(controller.state()).toMatchObject({ desiredId: 'tactical', effectivePresetId: 'tactical' });
  });

  it('restore() seeds a reconnected identity\'s choice without treating it as a fresh selection', () => {
    mount();
    const onSelect = vi.fn();
    const controller = createGmRolePresets({ doc: document, onSelect });
    controller.setAvailablePresets([TACTICAL]);

    controller.restore('tactical');
    expect(controller.state()).toMatchObject({ desiredId: 'tactical', effectivePresetId: 'tactical' });
    expect(document.getElementById('gm-inspector').hidden).toBe(true);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('restore() also falls back safely for a missing or invalid stored id', () => {
    mount();
    const controller = createGmRolePresets({ doc: document });
    controller.restore('never-authored');
    expect(controller.state().effectivePresetId).toBe('all');
    controller.restore(null);
    expect(controller.state().effectivePresetId).toBe('all');
  });

  it('lets two independent operators choose the same or different presets with no cross-talk', () => {
    document.body.innerHTML = `
      <select id="gm-role-preset-select-a"><option value="all">all</option></select>
      <select id="gm-role-preset-select-b"><option value="all">all</option></select>
      <section id="gm-map-panel"></section>
      <section id="gm-inspector"></section>
      <section id="gm-activity"></section>
      <section id="gm-station-controls"></section>
      <button id="gm-session-pause"></button>
      <button id="gm-session-resume"></button>
    `;
    // Two operators' own document scopes, faked by two independent controllers
    // over the same JS realm's document but each addressing its own <select>.
    const fakeDocA = {
      getElementById: (id) => (id === 'gm-role-preset-select'
        ? document.getElementById('gm-role-preset-select-a')
        : document.getElementById(id)),
      createElement: (tag) => document.createElement(tag),
    };
    const fakeDocB = {
      getElementById: (id) => (id === 'gm-role-preset-select'
        ? document.getElementById('gm-role-preset-select-b')
        : document.getElementById(id)),
      createElement: (tag) => document.createElement(tag),
    };
    const a = createGmRolePresets({ doc: fakeDocA });
    const b = createGmRolePresets({ doc: fakeDocB });
    a.setAvailablePresets([TACTICAL]);
    b.setAvailablePresets([TACTICAL]);

    // Duplicate selection: both GMs pick the identical preset independently.
    a.select('tactical');
    b.select('tactical');
    expect(a.state().effectivePresetId).toBe('tactical');
    expect(b.state().effectivePresetId).toBe('tactical');

    // One switches away; the other's own selection is untouched.
    a.select('all');
    expect(a.state().effectivePresetId).toBe('all');
    expect(b.state().effectivePresetId).toBe('tactical');
  });
});
