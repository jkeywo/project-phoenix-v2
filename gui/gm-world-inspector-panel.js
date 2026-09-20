import { renderInspectorReadOnly, validInspectorDescriptor } from './inspector-field.js';

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
  const retained = new Map();
  let history = []; let cursor = -1;
  const choose = (id, record = true) => {
    if (!id) return;
    if (record && history[cursor] !== id) {
      history = history.slice(0, cursor + 1); history.push(id);
      if (history.length > WORLD_INSPECTOR_HISTORY_LIMIT) {
        const dropped = history.shift();
        if (dropped !== selected && !history.includes(dropped)) retained.delete(dropped);
      }
      cursor = history.length - 1;
    }
    selected = id; render();
  };
  const render = () => {
    const live = domain?.readings.get(selected);
    if (live) { retained.set(selected, live); gone = false; }
    else gone = !!selected && retained.has(selected);
    const reading = live || retained.get(selected);
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
    back.disabled = cursor <= 0; forward.disabled = cursor < 0 || cursor >= history.length - 1;
  };
  select?.addEventListener('change', () => choose(select.value));
  back?.addEventListener('click', () => { if (cursor > 0) { cursor -= 1; selected = history[cursor]; render(); status.focus(); } });
  forward?.addEventListener('click', () => { if (cursor + 1 < history.length) { cursor += 1; selected = history[cursor]; render(); status.focus(); } });
  return {
    update(raw) { const next = parseGmWorldInspectorPayload(raw); if (!next) return false; domain = next; if (!selected) choose(next.readings.keys().next().value); else render(); return true; },
    select: choose,
    reset() { domain = null; selected = null; retained.clear(); gone = false; history = []; cursor = -1; render(); },
    state: () => ({ selected, gone, history: [...history], cursor }),
  };
}
