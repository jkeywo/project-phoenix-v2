// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { createGmWorldInspectorPanel, parseGmWorldInspectorPayload, WORLD_INSPECTOR_HISTORY_LIMIT } from '../../gui/gm-world-inspector-panel.js';
import { focusGmWorldInspectorOwner } from '../../gui/gm-workspace.js';

const descriptor = (id, mutability = 'recreate-required', action_panel = null) => ({
  id, label: id, group: 'scenario', kind: 'string', default_source: null,
  live_mutability: mutability, origin: { schema_path: id, document: null, line: null, layer: null },
  validation: [], ...(action_panel ? { action_panel } : {}),
});

it('routes event and Objective links to the exact established control owner', () => {
  document.body.innerHTML = markup;
  const shell = { showLog: vi.fn() };
  const mission = { focusEvent: vi.fn(() => true) };
  const objective = { focusObjective: vi.fn(() => true) };
  const panel = createGmWorldInspectorPanel({ doc: document, focusPanel: (owner, target) =>
    focusGmWorldInspectorOwner({ panel: owner, target, doc: document, shell, mission, objective }) });
  const linked = payload({ root: { label: 'Root', values: {
    'event[base-world::arrival].fire': 'true', 'objective[rescue].status': 'Active',
  } } });
  linked.world_inspector.fields.push(descriptor('objective[].status', 'named-action', 'objective'));
  panel.update(linked);

  document.querySelector('[data-field="event[base-world::arrival].fire"] button').click();
  document.querySelector('[data-field="objective[rescue].status"] button').click();

  expect(shell.showLog.mock.calls).toEqual([['gm-mission-panel'], ['gm-objective-panel']]);
  expect(mission.focusEvent).toHaveBeenCalledWith('base-world::arrival');
  expect(objective.focusObjective).toHaveBeenCalledWith('rescue');
});
const payload = readings => ({ world_inspector: { fields: [
  descriptor('global.title'), descriptor('flag[].value', 'derived'),
  descriptor('event[].fire', 'named-action', 'mission'),
], readings } });
const markup = `<section><select id="gm-world-fields-layer"></select><button id="gm-world-fields-back"></button><button id="gm-world-fields-forward"></button><p id="gm-world-fields-empty"></p><article id="gm-world-fields-card"><p id="gm-world-fields-status" tabindex="-1"></p><div id="gm-world-fields-list"></div></article></section>`;

describe('world/scenario Live Inspector', () => {
  it('rejects unclassified fields and never gives Flags an action', () => {
    expect(parseGmWorldInspectorPayload(payload({ root: { label: 'Root', values: {} } }))).not.toBeNull();
    const bad = payload({}); bad.world_inspector.fields[0].live_mutability = 'editable';
    expect(parseGmWorldInspectorPayload(bad)).toBeNull();
    const unknown = payload({ root: { label: 'Root', values: { 'unknown.path': 'x' } } });
    expect(parseGmWorldInspectorPayload(unknown)).toBeNull();
    expect(payload({}).world_inspector.fields.find(row => row.id === 'flag[].value').action_panel).toBeUndefined();
  });

  it('links named actions, freezes an unloaded layer, and retraces bounded history', () => {
    document.body.innerHTML = markup; const focusPanel = vi.fn();
    const panel = createGmWorldInspectorPanel({ doc: document, t: (id, p) => p?.layer || id, focusPanel });
    panel.update(payload({ root: { label: 'Root', values: { 'global.title': 'Probe', 'event[e].fire': 'true' } },
      layer: { label: 'Layer', origin_layer: 'assets/worlds/layer.toml', values: { 'flag[armed].value': '1' } } }));
    panel.select('layer'); panel.select('root');
    document.querySelector('[data-field="event[e].fire"] button').click();
    expect(focusPanel).toHaveBeenCalledWith('mission', 'e');
    panel.select('layer');
    panel.update(payload({ root: { label: 'Root', values: { 'global.title': 'Probe' } } }));
    expect(panel.state().gone).toBe(true);
    expect(document.getElementById('gm-world-fields-status').dataset.gone).toBe('true');
    expect(document.querySelector('#gm-world-fields-layer option:checked').disabled).toBe(true);
    expect(document.querySelector('[data-inspector-metadata]').textContent).toContain('assets/worlds/layer.toml');
    document.getElementById('gm-world-fields-back').click();
    document.getElementById('gm-world-fields-back').click();
    expect(document.querySelector('[data-field="flag[armed].value"] input').value).toBe('1');
    expect(document.activeElement).toBe(document.getElementById('gm-world-fields-status'));
    for (let i = 0; i < WORLD_INSPECTOR_HISTORY_LIMIT + 5; i += 1) panel.select(i % 2 ? 'root' : 'layer');
    expect(panel.state().history.length).toBeLessThanOrEqual(WORLD_INSPECTOR_HISTORY_LIMIT);
  });
});
