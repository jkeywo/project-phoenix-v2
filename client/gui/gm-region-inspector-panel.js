/** Read-only Region Live Inspector (issue #1492). */
import { renderInspectorReadOnly, validInspectorDescriptor } from './inspector-field.js';

export const GM_REGION_INSPECTOR_HISTORY_LIMIT = 20;
const GROUPS = ['identity', 'transform', 'shape', 'effect', 'presentation', 'runtime'];
const text = value => typeof value === 'string';

export function parseGmRegionInspectorPayload(payload) {
  let value = payload;
  if (typeof value === 'string') { try { value = JSON.parse(value); } catch (_) { return null; } }
  const domain = value?.region_inspector;
  if (!domain || !Array.isArray(domain.fields) || !domain.readings || typeof domain.readings !== 'object'
    || Array.isArray(domain.readings)) return null;
  const fields = [], ids = new Set();
  for (const field of domain.fields) {
    if (!field || !text(field.id) || !text(field.label) || !GROUPS.includes(field.group)
      || !validInspectorDescriptor(field) || field.id !== field.origin.schema_path
      || field.live_mutability === 'named-action' || ids.has(field.id)) return null;
    ids.add(field.id); fields.push({ ...field, descriptor: field });
  }
  const readings = new Map();
  for (const [id, reading] of Object.entries(domain.readings)) {
    if (!text(id) || !reading || !text(reading.label) || !reading.values
      || typeof reading.values !== 'object' || Array.isArray(reading.values)
      || !Array.isArray(reading.occupants)) return null;
    const values = new Map();
    for (const [key, item] of Object.entries(reading.values)) {
      if (!ids.has(key) || !text(item)) return null;
      values.set(key, item);
    }
    const occupants = [];
    for (const occupant of reading.occupants) {
      if (!occupant || !text(occupant.entity_id) || !text(occupant.label)
        || !Array.isArray(occupant.consequences)) return null;
      const consequences = [];
      for (const consequence of occupant.consequences) {
        if (!consequence || !text(consequence.kind) || !consequence.values
          || typeof consequence.values !== 'object' || Array.isArray(consequence.values)
          || !Object.values(consequence.values).every(text)) return null;
        consequences.push({ kind: consequence.kind, values: { ...consequence.values } });
      }
      occupants.push({ entity_id: occupant.entity_id, label: occupant.label, consequences });
    }
    readings.set(id, { label: reading.label, values, occupants });
  }
  const liveIds = new Set();
  if (value.entities != null) {
    if (!Array.isArray(value.entities)) return null;
    for (const entity of value.entities) {
      if (!entity || !text(entity.entity_id)) return null;
      liveIds.add(entity.entity_id);
    }
  }
  return { fields, readings, liveIds };
}

const emptyDomain = () => ({ fields: [], readings: new Map(), liveIds: new Set() });

export function createGmRegionInspectorPanel({ doc = globalThis.document, t = id => id } = {}) {
  const el = suffix => doc?.getElementById(`gm-region-fields-${suffix}`);
  let domain = emptyDomain(), selected = null, retained = null, gone = false;
  const retainedById = new Map(), rendered = new Map(), history = []; let cursor = -1;

  function groupNode(group) {
    let node = el(`group-${group}`); if (node) return node;
    const list = el('list'); if (!list) return null;
    node = doc.createElement('section'); node.id = `gm-region-fields-group-${group}`;
    const heading = doc.createElement('h3'); heading.id = `${node.id}-heading`;
    heading.textContent = t(`inspector.region.group.${group}`);
    node.setAttribute('role', 'group'); node.setAttribute('aria-labelledby', heading.id);
    node.append(heading); list.append(node); return node;
  }
  function fieldNode(field) {
    let node = rendered.get(field.id); if (node?.isConnected) return node;
    node = doc.createElement('div'); node.dataset.field = field.id;
    const label = doc.createElement('span'); label.textContent = `${t(field.label)} — ${field.id}`;
    node.append(label); groupNode(field.group)?.append(node); rendered.set(field.id, node); return node;
  }
  function push(id) {
    if (!id || history[cursor] === id) return;
    history.splice(cursor + 1); history.push(id);
    if (history.length > GM_REGION_INSPECTOR_HISTORY_LIMIT) history.shift();
    cursor = history.length - 1;
  }
  function renderOccupants() {
    const list = el('occupants'); if (!list) return;
    list.replaceChildren();
    for (const occupant of retained?.occupants || []) {
      const item = doc.createElement('li');
      const heading = doc.createElement('h4'); heading.textContent = `${occupant.label} (${occupant.entity_id})`;
      item.append(heading);
      if (!occupant.consequences.length) {
        const none = doc.createElement('p'); none.textContent = t('inspector.region.consequence.none'); item.append(none);
      }
      for (const consequence of occupant.consequences) {
        const section = doc.createElement('section');
        const title = doc.createElement('h5'); title.textContent = t(`inspector.region.consequence.${consequence.kind}`);
        const details = doc.createElement('dl');
        for (const [key, value] of Object.entries(consequence.values)) {
          const term = doc.createElement('dt'); term.textContent = t(`inspector.region.consequence.field.${key}`);
          const description = doc.createElement('dd'); description.textContent = value;
          details.append(term, description);
        }
        section.append(title, details); item.append(section);
      }
      list.append(item);
    }
  }
  function render() {
    if (el('empty')) el('empty').hidden = !!retained;
    if (el('card')) el('card').hidden = !retained;
    if (el('status')) {
      el('status').textContent = !retained ? '' : t(gone ? 'inspector.region.subject_gone' : 'inspector.region.subject', { name: retained.label });
      el('status').dataset.gone = String(gone);
    }
    for (const field of domain.fields) {
      const node = fieldNode(field), value = retained?.values.get(field.id);
      node.hidden = !retained || value === undefined; if (node.hidden) continue;
      let slot = node.querySelector('[data-inspector-value]');
      if (!slot) { slot = doc.createElement('div'); slot.dataset.inspectorValue = ''; node.append(slot); }
      renderInspectorReadOnly(slot, value, field.descriptor, { t, label: field.label });
    }
    renderOccupants();
    if (el('back')) el('back').disabled = cursor <= 0;
    if (el('forward')) el('forward').disabled = cursor < 0 || cursor >= history.length - 1;
  }
  function show(id, { record = true, focus = false } = {}) {
    selected = id || null; if (record) push(selected);
    const reading = selected ? domain.readings.get(selected) : null;
    if (reading) { retained = reading; retainedById.set(selected, reading); gone = false; }
    else if (!selected || domain.liveIds.has(selected)) { retained = null; gone = false; }
    else { retained = retainedById.get(selected) || null; gone = retained != null; }
    render(); if (focus) el('status')?.focus?.();
  }
  el('back')?.addEventListener('click', () => { if (cursor > 0) { cursor--; show(history[cursor], { record: false, focus: true }); } });
  el('forward')?.addEventListener('click', () => { if (cursor >= 0 && cursor < history.length - 1) { cursor++; show(history[cursor], { record: false, focus: true }); } });
  return {
    update(payload) {
      const parsed = parseGmRegionInspectorPayload(payload); if (!parsed) return false;
      for (const [id, reading] of parsed.readings) retainedById.set(id, reading);
      domain = parsed; show(selected, { record: false }); return true;
    },
    select(entity) { show(entity?.entity_id || null); },
    reset() { domain = emptyDomain(); selected = null; retained = null; gone = false; retainedById.clear(); history.length = 0; cursor = -1; rendered.clear(); el('list')?.replaceChildren(); render(); },
    state: () => ({ selected, gone, history: [...history], cursor, fields: domain.fields.length, occupants: retained?.occupants.length || 0 }),
  };
}
