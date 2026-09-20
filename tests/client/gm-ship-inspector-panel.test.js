// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { createGmShipInspectorPanel, parseGmShipInspectorPayload, GM_SHIP_INSPECTOR_HISTORY_LIMIT } from '../../gui/gm-ship-inspector-panel.js';

const source = readFileSync('server.html', 'utf8');
const start = source.indexOf('<section id="gm-ship-fields-panel"');
const markup = source.slice(start, source.indexOf('</section>', source.indexOf('<article id="gm-ship-fields-card"', start)) + 10);
const descriptor = (id, group, live_mutability, action_panel = undefined) => ({
  id, label: 'inspector.ship.field', group, kind: 'string', live_mutability,
  origin: { schema_path: id, document: null, line: null, layer: null }, validation: [],
  ...(action_panel ? { action_panel } : {}),
});
const fields = [
  descriptor('station[helm].name', 'station', 'recreate-required'),
  descriptor('runtime.station[helm].puppet', 'runtime', 'named-action', 'station'),
  descriptor('runtime.system[engine].gm_disabled', 'runtime', 'derived'),
  descriptor('runtime.system[engine].availability_action', 'runtime', 'named-action', 'system'),
  descriptor('runtime.system[engine].health.current_hp', 'runtime', 'derived'),
  descriptor('runtime.system[engine].effect_action', 'runtime', 'named-action', 'effect'),
  descriptor('runtime.system[engine].state.HelmEngine.cooldown', 'runtime', 'derived'),
];
const reading = (label = 'Cruiser', destroyed = false) => ({ label, destroyed, values: Object.fromEntries(fields.map(field => [field.id, field.id.includes('name') ? 'Helm' : '0'])) });
const payload = (readings, entities = undefined) => ({
  ship_inspector: { fields, readings },
  ...(entities ? { entities: entities.map(entity_id => ({ entity_id })) } : {}),
});
const t = (id, params) => params ? `${id} ${Object.values(params).join(' ')}` : id;

function mount(options = {}) {
  document.body.innerHTML = markup;
  return createGmShipInspectorPanel({ doc: document, t, ...options });
}

describe('hull/Station/System Live Inspector', () => {
  beforeEach(() => { document.body.innerHTML = ''; });

  it('rejects unclassified fields and renders every active known/runtime leaf read-only', () => {
    expect(parseGmShipInspectorPayload(payload({ ship: reading() })).fields).toHaveLength(fields.length);
    const bad = structuredClone(payload({})); bad.ship_inspector.fields[0].live_mutability = 'editable';
    expect(parseGmShipInspectorPayload(bad)).toBeNull();
    const panel = mount(); panel.update(payload({ ship: reading() })); panel.select({ entity_id: 'ship' });
    expect(document.querySelectorAll('#gm-ship-fields-list input:not([disabled])')).toHaveLength(0);
    expect([...document.querySelectorAll('[data-field]')].find(node => node.dataset.field === 'runtime.system[engine].state.HelmEngine.cooldown').querySelector('input').value).toBe('0');
  });

  it('aims each link at the established matching control', () => {
    const focusEffect = vi.fn(), focusSystem = vi.fn(), focusStation = vi.fn();
    const panel = mount({ focusEffect, focusSystem, focusStation });
    panel.update(payload({ ship: reading() })); panel.select({ entity_id: 'ship' });
    const row = id => [...document.querySelectorAll('[data-field]')].find(node => node.dataset.field === id);
    row('runtime.system[engine].effect_action').querySelector('button').click();
    row('runtime.system[engine].availability_action').querySelector('button').click();
    row('runtime.station[helm].puppet').querySelector('button').click();
    expect(focusEffect).toHaveBeenCalledWith('ship', 'system:engine');
    expect(focusSystem).toHaveBeenCalledWith('ship', 'engine');
    expect(focusStation).toHaveBeenCalledWith('ship', 'helm');
  });

  it('freezes missing and destroyed readings and disables every action', () => {
    const panel = mount({ focusEffect: vi.fn(), focusSystem: vi.fn(), focusStation: vi.fn() });
    panel.update(payload({ ship: reading() })); panel.select({ entity_id: 'ship' });
    panel.update(payload({}));
    expect(panel.state().gone).toBe(true);
    expect([...document.querySelectorAll('[data-inspector-action]')].every(button => button.disabled)).toBe(true);
    panel.update(payload({ wreck: reading('Wreck', true) })); panel.select({ entity_id: 'wreck' });
    expect(panel.state().destroyed).toBe(true);
    expect([...document.querySelectorAll('[data-inspector-action]')].every(button => button.disabled)).toBe(true);
  });

  it('clears a live non-ship without misreporting the previous hull as gone', () => {
    const panel = mount();
    panel.update(payload({ ship: reading() }, ['ship', 'beacon']));
    panel.select({ entity_id: 'ship' });
    panel.select({ entity_id: 'beacon' });
    expect(panel.state()).toMatchObject({ selected: 'beacon', gone: false, destroyed: false });
    expect(document.getElementById('gm-ship-fields-card').hidden).toBe(true);
    expect(document.getElementById('gm-ship-fields-status').textContent).toBe('');
    document.getElementById('gm-ship-fields-back').click();
    expect(panel.state()).toMatchObject({ selected: 'ship', gone: false });
  });

  it('retains each hull independently and freezes its first destroyed reading', () => {
    const panel = mount();
    panel.update(payload({ a: reading('A'), b: reading('B') }, ['a', 'b']));
    panel.select({ entity_id: 'a' });
    panel.select({ entity_id: 'b' });
    panel.update(payload({ b: reading('B wreck', true) }, ['b']));
    expect(panel.state()).toMatchObject({ selected: 'b', destroyed: true, gone: false });
    panel.update(payload({ b: reading('B impossible recovery', false) }, ['b']));
    expect(panel.state().destroyed).toBe(true);
    document.getElementById('gm-ship-fields-back').click();
    expect(panel.state()).toMatchObject({ selected: 'a', gone: true, destroyed: false });
  });

  it('keeps bounded Back/Forward identity history and restores focus', () => {
    const panel = mount(); panel.update(payload({ a: reading('A'), b: reading('B') }));
    for (let index = 0; index < GM_SHIP_INSPECTOR_HISTORY_LIMIT + 5; index++) panel.select({ entity_id: index % 2 ? 'a' : 'b' });
    expect(panel.state().history).toHaveLength(GM_SHIP_INSPECTOR_HISTORY_LIMIT);
    document.getElementById('gm-ship-fields-back').click();
    expect(document.activeElement).toBe(document.getElementById('gm-ship-fields-status'));
    expect(document.getElementById('gm-ship-fields-forward').disabled).toBe(false);
  });

  it('is folded, selection-scoped, reset and registered for browser/native parity', () => {
    const workspace = readFileSync('gui/gm-workspace.js', 'utf8');
    expect(workspace).toContain('gmShipFields.update(p)');
    expect(workspace).toContain('gmShipFields.select(entity)');
    expect(workspace).toContain('gmShipFields.reset()');
    expect(workspace.indexOf("shell.temporaryActions?.open('effect')"))
      .toBeLessThan(workspace.indexOf('gmDirectEffect.selectScope(scope)'));
    expect(readFileSync('src/native_host/native_gm/document.rs', 'utf8')).toContain('gm-ship-fields-panel');
  });
});
