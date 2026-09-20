/** Hull, Station and System Live Inspector (issue #1491). */
import { renderInspectorReadOnly, validInspectorDescriptor } from './inspector-field.js';

export const GM_SHIP_INSPECTOR_HISTORY_LIMIT = 20;
const GROUPS = ['hull', 'station', 'system', 'power', 'runtime'];
const text = value => typeof value === 'string';

export function parseGmShipInspectorPayload(payload) {
  let value = payload;
  if (typeof value === 'string') { try { value = JSON.parse(value); } catch (_) { return null; } }
  const domain = value?.ship_inspector;
  if (!domain || !Array.isArray(domain.fields) || !domain.readings || typeof domain.readings !== 'object' || Array.isArray(domain.readings)) return null;
  const fields = [], ids = new Set();
  for (const row of domain.fields) {
    if (!row || !text(row.id) || !text(row.label) || !GROUPS.includes(row.group)
      || !validInspectorDescriptor(row) || row.id !== row.origin.schema_path || ids.has(row.id)
      || (row.action_panel != null && !['effect', 'system', 'station'].includes(row.action_panel))) return null;
    ids.add(row.id); fields.push({ ...row, descriptor: row });
  }
  const readings = new Map();
  for (const [id, reading] of Object.entries(domain.readings)) {
    if (!text(id) || !reading || !text(reading.label) || typeof reading.destroyed !== 'boolean'
      || !reading.values || typeof reading.values !== 'object' || Array.isArray(reading.values)) return null;
    const values = new Map();
    for (const [key, item] of Object.entries(reading.values)) {
      if (!ids.has(key) || !text(item)) return null;
      values.set(key, item);
    }
    readings.set(id, { label: reading.label, destroyed: reading.destroyed, values });
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
const bracketId = (path, kind) => path.match(new RegExp(`(?:runtime\\.)?${kind}\\[([^\\]]+)\\]`))?.[1] || null;

export function createGmShipInspectorPanel({ doc = globalThis.document, t = id => id,
  focusEffect = null, focusSystem = null, focusStation = null } = {}) {
  const el = suffix => doc?.getElementById(`gm-ship-fields-${suffix}`);
  let domain = emptyDomain(), selected = null, retained = null, gone = false;
  const retainedById = new Map();
  const history = []; let cursor = -1; const rendered = new Map();

  function groupNode(group) {
    let node = el(`group-${group}`); if (node) return node;
    const list = el('list'); if (!list) return null;
    node = doc.createElement('section'); node.id = `gm-ship-fields-group-${group}`;
    const heading = doc.createElement('h3'); heading.id = `${node.id}-heading`; heading.textContent = t(`inspector.ship.group.${group}`);
    node.setAttribute('role', 'group'); node.setAttribute('aria-labelledby', heading.id); node.append(heading); list.append(node); return node;
  }
  function fieldNode(field) {
    let node = rendered.get(field.id); if (node?.isConnected) return node;
    node = doc.createElement('div'); node.dataset.field = field.id;
    const label = doc.createElement('span'); label.textContent = `${t(field.label)} — ${field.id}`; node.append(label);
    groupNode(field.group)?.append(node); rendered.set(field.id, node); return node;
  }
  function push(id) {
    if (!id || history[cursor] === id) return;
    history.splice(cursor + 1); history.push(id);
    if (history.length > GM_SHIP_INSPECTOR_HISTORY_LIMIT) history.shift(); cursor = history.length - 1;
  }
  function action(node, field) {
    let button = node.querySelector('[data-inspector-action]');
    if (!field.action_panel) { if (button) button.remove(); return; }
    if (!button) { button = doc.createElement('button'); button.type = 'button'; button.dataset.inspectorAction = field.id; node.append(button); }
    button.textContent = t(`inspector.ship.open_${field.action_panel}`);
    const system = bracketId(field.id, 'system'); const station = bracketId(field.id, 'station');
    const owner = field.action_panel === 'effect' ? focusEffect : field.action_panel === 'system' ? focusSystem : focusStation;
    button.disabled = gone || retained?.destroyed === true || typeof owner !== 'function';
    button.onclick = () => {
      if (button.disabled) return;
      if (field.action_panel === 'effect') owner(selected, system ? `system:${system}` : 'entity');
      else if (field.action_panel === 'system') owner(selected, system);
      else owner(selected, station);
    };
  }
  function render() {
    const empty = el('empty'), card = el('card'), status = el('status');
    if (empty) empty.hidden = !!retained; if (card) card.hidden = !retained;
    if (status) { status.textContent = !retained ? '' : t(gone ? 'inspector.ship.subject_gone' : retained.destroyed ? 'inspector.ship.subject_destroyed' : 'inspector.ship.subject', { name: retained.label || selected }); status.dataset.gone = String(gone || retained?.destroyed === true); }
    for (const field of domain.fields) {
      const node = fieldNode(field), value = retained?.values.get(field.id); node.hidden = !selected || value === undefined; if (node.hidden) continue;
      let slot = node.querySelector('[data-inspector-value]'); if (!slot) { slot = doc.createElement('div'); slot.dataset.inspectorValue = ''; node.append(slot); }
      renderInspectorReadOnly(slot, value, field.descriptor, { t, label: field.label });
      const input = slot.querySelector('input'); if (input) { input.disabled = true; input.readOnly = true; }
      action(node, field);
    }
    if (el('back')) el('back').disabled = cursor <= 0;
    if (el('forward')) el('forward').disabled = cursor < 0 || cursor >= history.length - 1;
  }
  function show(id, { record = true, focus = false } = {}) {
    selected = id || null; if (record) push(selected); const reading = selected ? domain.readings.get(selected) : null;
    if (reading) {
      const prior = retainedById.get(selected);
      retained = prior?.destroyed ? prior : reading;
      retainedById.set(selected, retained); gone = false;
    } else if (!selected || domain.liveIds.has(selected)) {
      // A live entity with no ship reading is simply outside this domain. It
      // must not inherit the previously selected hull's stale card.
      retained = null; gone = false;
    } else {
      retained = retainedById.get(selected) || null;
      gone = retained != null;
    }
    render(); if (focus) el('status')?.focus?.();
  }
  el('back')?.addEventListener('click', () => { if (cursor > 0) { cursor--; show(history[cursor], { record: false, focus: true }); } });
  el('forward')?.addEventListener('click', () => { if (cursor >= 0 && cursor < history.length - 1) { cursor++; show(history[cursor], { record: false, focus: true }); } });
  return {
    update(payload) {
      const parsed = parseGmShipInspectorPayload(payload); if (!parsed) return false;
      // Destruction is the terminal identity state for this reading. Keep the
      // first destroyed snapshot even if a later projection still happens to
      // carry the ECS entity while cleanup completes.
      for (const [id, reading] of parsed.readings) {
        if (!retainedById.get(id)?.destroyed) retainedById.set(id, reading);
      }
      domain = parsed; show(selected, { record: false }); return true;
    },
    select(entity) { show(entity?.entity_id || null); },
    reset() { domain = emptyDomain(); selected = null; retained = null; gone = false; retainedById.clear(); history.length = 0; cursor = -1; rendered.clear(); el('list')?.replaceChildren(); render(); },
    state: () => ({ selected, gone, destroyed: retained?.destroyed === true, history: [...history], cursor, fields: domain.fields.length }),
  };
}
