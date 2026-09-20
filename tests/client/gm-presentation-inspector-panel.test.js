import { beforeEach, describe, expect, it, vi } from 'vitest';
import { JSDOM } from 'jsdom';
import { createGmPresentationInspectorPanel, parseGmPresentationInspectorPayload,
  GM_PRESENTATION_INSPECTOR_HISTORY_LIMIT } from '../../gui/gm-presentation-inspector-panel.js';

const descriptor = (id, mutability = 'recreate-required', action = null) => ({ id,
  label: 'inspector.presentation.field', group: id.split('.')[0], kind: 'string',
  live_mutability: mutability, origin: { schema_path: id, document: null, line: null, layer: null },
  validation: mutability === 'recreate-required' ? ['inspector.presentation.recreate_explanation'] : [],
  ...(action ? { action_panel: action } : {}) });
const payload = () => ({ presentation_inspector: {
  fields: [descriptor('views.camera[].name', 'named-action', 'presentation'),
    descriptor('runtime.card.kind', 'derived'), descriptor('sound.id')],
  readings: {
    'ship:a': { label: 'Phoenix', kind: 'ship', ship_id: 'a', action_available: true,
      values: { 'views.camera[0].name': 'camera_fore', 'runtime.card.kind': 'title' } },
    'sound:bell': { label: 'Bell', kind: 'sound', action_available: false,
      values: { 'sound.id': 'bell' } },
  },
} });

beforeEach(() => {
  const dom = new JSDOM(`<select id="gm-presentation-fields-subject"></select>
    <button id="gm-presentation-fields-back"></button><button id="gm-presentation-fields-forward"></button>
    <p id="gm-presentation-fields-empty"></p><article id="gm-presentation-fields-card">
    <p id="gm-presentation-fields-status" tabindex="-1"></p><div id="gm-presentation-fields-list"></div></article>`);
  globalThis.document = dom.window.document;
});

describe('presentation/audio Live Inspector', () => {
  it('strictly parses dynamic camera rows and rejects a duplicated mutation owner', () => {
    expect(parseGmPresentationInspectorPayload(payload()).readings.size).toBe(2);
    const bad = payload(); bad.presentation_inspector.fields[0].action_panel = 'audio';
    expect(parseGmPresentationInspectorPayload(bad)).toBeNull();
  });

  it('renders structured non-colour rows and focuses the established action owner', () => {
    const focusPresentation = vi.fn();
    const panel = createGmPresentationInspectorPanel({ doc: document, t: (id, vars) => vars?.name || id, focusPresentation });
    expect(panel.update(payload())).toBe(true);
    const row = document.querySelector('[data-field="views.camera[0].name"]');
    expect(row.querySelector('input').value).toBe('camera_fore');
    row.querySelector('[data-inspector-action]').click();
    expect(focusPresentation).toHaveBeenCalledWith(expect.objectContaining({ field: 'views.camera[0].name', value: 'camera_fore' }));
    document.getElementById('gm-presentation-fields-subject').value = 'sound:bell';
    document.getElementById('gm-presentation-fields-subject').dispatchEvent(new document.defaultView.Event('change'));
    expect(document.querySelector('[data-field="sound.id"] input').disabled).toBe(true);
  });

  it('freezes an unloaded definition as gone and bounds keyboard-return history', () => {
    const panel = createGmPresentationInspectorPanel({ doc: document, t: (id, vars) => vars?.name || id });
    panel.update(payload());
    panel.update({ presentation_inspector: { ...payload().presentation_inspector, readings: {} } });
    expect(panel.state().gone).toBe(true);
    expect(document.querySelector('[data-inspector-action]').disabled).toBe(true);
    panel.reset();
    const many = payload(); many.presentation_inspector.readings = Object.fromEntries(
      Array.from({ length: GM_PRESENTATION_INSPECTOR_HISTORY_LIMIT + 5 }, (_, i) => [`ship:s${i}`,
        { label: `Ship ${i}`, kind: 'ship', ship_id: `s${i}`, action_available: true,
          values: { 'views.camera[0].name': 'camera_fore' } }]));
    panel.update(many);
    const select = document.getElementById('gm-presentation-fields-subject');
    for (const id of Object.keys(many.presentation_inspector.readings)) {
      select.value = id; select.dispatchEvent(new document.defaultView.Event('change'));
    }
    expect(panel.state().history).toHaveLength(GM_PRESENTATION_INSPECTOR_HISTORY_LIMIT);
    document.getElementById('gm-presentation-fields-back').click();
    expect(document.activeElement.id).toBe('gm-presentation-fields-status');
  });
});
