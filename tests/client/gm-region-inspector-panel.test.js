// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import {
  createGmRegionInspectorPanel,
  GM_REGION_INSPECTOR_HISTORY_LIMIT,
  parseGmRegionInspectorPayload,
} from '../../gui/gm-region-inspector-panel.js';

const source = readFileSync('server.html', 'utf8');
const start = source.indexOf('<section id="gm-region-fields-panel"');
const end = source.indexOf('</article>', start) + '</article>'.length;
const markup = source.slice(start, end);
const descriptor = (id, group, live_mutability = 'recreate-required') => ({
  id, label: 'inspector.region.field', group, kind: 'string', live_mutability,
  origin: { schema_path: id, document: null, line: null, layer: null },
  validation: live_mutability === 'recreate-required' ? ['inspector.region.recreate_explanation'] : [],
});
const fields = [
  descriptor('identity.kind', 'identity'),
  descriptor('shape.type', 'shape'),
  descriptor('effects.slow_zone.thrust_modifier', 'effect'),
  descriptor('presentation.radar.region_colour.b', 'presentation'),
  descriptor('runtime.occupant_count', 'runtime', 'derived'),
];
const reading = (label = 'Ion storm') => ({
  label,
  values: Object.fromEntries(fields.map(field => [field.id,
    field.id === 'runtime.occupant_count' ? '1' : field.id === 'identity.kind' ? 'hazard' : 'value'])),
  occupants: [{ entity_id: 'ship', label: 'Scout', consequences: [
    { kind: 'slow-zone', values: { thrust_multiplier: '0.5', yaw_rate_multiplier: 'not-authored' } },
  ] }],
});
const payload = (readings, entities = undefined) => ({
  region_inspector: { fields, readings },
  ...(entities ? { entities: entities.map(entity_id => ({ entity_id })) } : {}),
});
const t = (id, params) => params ? `${id} ${Object.values(params).join(' ')}` : id;

function mount() {
  document.body.innerHTML = markup;
  return createGmRegionInspectorPanel({ doc: document, t });
}

describe('Region Live Inspector', () => {
  beforeEach(() => { document.body.innerHTML = ''; });

  it('accepts only fully classified read-only descriptors and renders recreate explanations', () => {
    expect(parseGmRegionInspectorPayload(payload({ region: reading() })).fields).toHaveLength(fields.length);
    for (const mutation of [
      value => { value.region_inspector.fields[0].live_mutability = 'named-action'; },
      value => { value.region_inspector.fields[0].origin.schema_path = 'other'; },
      value => { value.region_inspector.fields[0].group = 'private-cache'; },
    ]) {
      const bad = structuredClone(payload({})); mutation(bad);
      expect(parseGmRegionInspectorPayload(bad)).toBeNull();
    }
    const panel = mount(); panel.update(payload({ region: reading() })); panel.select({ entity_id: 'region' });
    expect(document.querySelectorAll('#gm-region-fields-list input:not([disabled])')).toHaveLength(0);
    expect(document.querySelectorAll('[data-inspector-action]')).toHaveLength(0);
    expect(document.querySelector('[data-field="shape.type"] [data-inspector-value]').textContent)
      .toContain('inspector.region.recreate_explanation');
  });

  it('renders occupants and their public effective consequences as structured accessible data', () => {
    const panel = mount(); panel.update(payload({ region: reading() })); panel.select({ entity_id: 'region' });
    expect(document.querySelector('#gm-region-fields-occupants h4').textContent).toContain('Scout (ship)');
    expect(document.querySelector('#gm-region-fields-occupants h5').textContent)
      .toBe('inspector.region.consequence.slow-zone');
    expect([...document.querySelectorAll('#gm-region-fields-occupants dt')].map(node => node.textContent))
      .toEqual(['inspector.region.consequence.field.thrust_multiplier',
        'inspector.region.consequence.field.yaw_rate_multiplier']);
    expect([...document.querySelectorAll('#gm-region-fields-occupants dd')].map(node => node.textContent))
      .toEqual(['0.5', 'not-authored']);
  });

  it('retains each unloaded Region independently but clears a live non-Region selection', () => {
    const panel = mount();
    panel.update(payload({ a: reading('A'), b: reading('B') }, ['a', 'b', 'beacon']));
    panel.select({ entity_id: 'a' }); panel.select({ entity_id: 'b' });
    panel.update(payload({ b: reading('B') }, ['b', 'beacon']));
    document.getElementById('gm-region-fields-back').click();
    expect(panel.state()).toMatchObject({ selected: 'a', gone: true });
    expect(document.getElementById('gm-region-fields-status').textContent).toContain('subject_gone');
    panel.select({ entity_id: 'beacon' });
    expect(panel.state()).toMatchObject({ selected: 'beacon', gone: false });
    expect(document.getElementById('gm-region-fields-card').hidden).toBe(true);
  });

  it('keeps bounded Back/Forward identity history and restores focus', () => {
    const panel = mount(); panel.update(payload({ a: reading('A'), b: reading('B') }));
    for (let index = 0; index < GM_REGION_INSPECTOR_HISTORY_LIMIT + 5; index++) {
      panel.select({ entity_id: index % 2 ? 'a' : 'b' });
    }
    expect(panel.state().history).toHaveLength(GM_REGION_INSPECTOR_HISTORY_LIMIT);
    document.getElementById('gm-region-fields-back').click();
    expect(document.activeElement).toBe(document.getElementById('gm-region-fields-status'));
    expect(document.getElementById('gm-region-fields-forward').disabled).toBe(false);
  });

  it('is selection-scoped, reset, and registered for browser/native parity', () => {
    const workspace = readFileSync('gui/gm-workspace.js', 'utf8');
    expect(workspace).toContain("updateReading('region-fields', gmRegionFields, p)");
    expect(workspace).toContain('gmRegionFields.select(entity)');
    expect(workspace).toContain('gmRegionFields.reset()');
    expect(readFileSync('src/native_host/native_gm/document.rs', 'utf8')).toContain('gm-region-fields-panel');
  });
});
