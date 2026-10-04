import { renderInspectorReadOnly, validInspectorDescriptor } from './inspector-field.js';
import { createInspectorSession, bindInspectorNavigation } from './inspector-session.js';

export const WORLD_INSPECTOR_HISTORY_LIMIT = 32;
const text = value => typeof value === 'string' ? value : '';
const schemaPath = value => value.replace(/\[[^\]]*\]/g, '[]');
const recordTarget = value => value.match(/^[^[]+\[([^\]]+)\]/)?.[1] || value;

export function parseGmWorldInspectorPayload(raw) {
  const domain = raw?.world_inspector;
  if (!domain || !Array.isArray(domain.fields) || !domain.readings
      || typeof domain.readings !== 'object' || Array.isArray(domain.readings)) return null;
  const seen = new Set();
  const fields = domain.fields.map(field => ({ ...field, id: text(field?.id), group: text(field?.group) }));
  for (const field of fields) {
    if (!field.id || !field.label || !field.group || seen.has(field.id)
        || field.origin?.schema_path !== field.id || !validInspectorDescriptor(field)
        || (field.live_mutability === 'named-action') !== !!text(field.action_panel)) return null;
    seen.add(field.id);
  }
  const readings = new Map();
  for (const [id, row] of Object.entries(domain.readings)) {
    if (!id || !row || typeof row !== 'object' || Array.isArray(row)
        || !row.values || typeof row.values !== 'object' || Array.isArray(row.values)
        || (row.origin_layer != null && typeof row.origin_layer !== 'string')) return null;
    const values = new Map();
    for (const [path, value] of Object.entries(row.values)) {
      if (!path || typeof value !== 'string'
          || !fields.some(field => schemaPath(path) === field.id)) return null;
      values.set(path, value);
    }
    readings.set(id, { label: text(row.label) || id, originLayer: row.origin_layer || null, values });
  }
  return { fields, readings };
}

export function createGmWorldInspectorPanel({ doc = document, t = id => id, focusPanel = () => {} } = {}) {
  const select = doc.getElementById('gm-world-fields-layer');
  const list = doc.getElementById('gm-world-fields-list');
  const card = doc.getElementById('gm-world-fields-card');
  const empty = doc.getElementById('gm-world-fields-empty');
  const status = doc.getElementById('gm-world-fields-status');
  const back = doc.getElementById('gm-world-fields-back');
  const forward = doc.getElementById('gm-world-fields-forward');
  let domain = null; let selected = null; let gone = false;
  const session = createInspectorSession(WORLD_INSPECTOR_HISTORY_LIMIT);
  const choose = (id, record = true) => {
    if (!id) return;
    session.forget(session.select(id, { record }));
    render();
  };
  const render = () => {
    selected = session.selected;
    refreshNavigation();
    const live = domain?.readings.get(selected);
    if (live) { session.remember(selected, live); gone = false; }
    else gone = !!selected && session.reading(selected) != null;
    const reading = live || session.reading(selected);
    if (select && domain) {
      const options = [...domain.readings].map(([id, row]) => Object.assign(doc.createElement('option'), {
        value: id, textContent: row.label, selected: id === selected,
      }));
      if (gone) options.push(Object.assign(doc.createElement('option'), {
        value: selected, textContent: reading.label, selected: true, disabled: true,
      }));
      select.replaceChildren(...options);
    }
    if (empty) empty.hidden = !!reading; if (card) card.hidden = !reading;
    if (!reading) return;
    status.textContent = gone ? t('inspector.world.gone', { layer: reading.label }) : t('inspector.world.reading', { layer: reading.label });
    status.dataset.gone = String(gone);
    list.replaceChildren();
    for (const field of domain.fields) {
      const matches = [...reading.values.entries()].filter(([id]) => schemaPath(id) === field.id);
      if (!matches.length) continue;
      for (const [id, value] of matches) {
        const row = doc.createElement('div'); row.className = 'gm-inspector-field'; row.dataset.field = id;
        const label = doc.createElement('label');
        const translated = t(field.label);
        label.textContent = translated && translated !== field.label
          ? translated : id.replaceAll('_', ' ').replaceAll('.', ' › ');
        const displayLabel = label.textContent;
        const descriptor = reading.originLayer
          ? { ...field, origin: { ...field.origin, layer: reading.originLayer } } : field;
        const slot = doc.createElement('div'); renderInspectorReadOnly(slot, value, descriptor, { t, label: field.label });
        slot.querySelector('input')?.setAttribute('aria-label', displayLabel);
        row.append(label, slot);
        if (field.action_panel) {
          const action = doc.createElement('button'); action.type = 'button'; action.textContent = t('inspector.world.open_action');
          action.disabled = gone;
          action.addEventListener('click', () => focusPanel(field.action_panel, recordTarget(id)));
          row.append(action);
        }
        list.append(row);
      }
    }
  };
  select?.addEventListener('change', () => choose(select.value));
  const refreshNavigation = bindInspectorNavigation({ session, back, forward, status, render });
  return {
    update(raw) { const next = parseGmWorldInspectorPayload(raw); if (!next) return false; domain = next; if (!selected) choose(next.readings.keys().next().value); else render(); return true; },
    select: choose,
    reset() { domain = null; selected = null; session.reset(); gone = false; render(); },
    state: () => ({ ...session.state(), gone }),
  };
}
