/** Authored Viewscreen presentation/audio Live Inspector (#1493). */
import { renderInspectorReadOnly, validInspectorDescriptor } from './inspector-field.js';
import { createInspectorSession, bindInspectorNavigation } from './inspector-session.js';

export const GM_PRESENTATION_INSPECTOR_HISTORY_LIMIT = 20;
const GROUPS = ['identity', 'provenance', 'views', 'cue', 'comms', 'runtime', 'catalog', 'asset', 'sound'];
const text = value => typeof value === 'string';
const schemaPath = value => value.replace(/\[[^\]]*\]/g, '[]');

export function parseGmPresentationInspectorPayload(payload) {
  let value = payload;
  if (typeof value === 'string') { try { value = JSON.parse(value); } catch (_) { return null; } }
  const domain = value?.presentation_inspector;
  if (!domain || !Array.isArray(domain.fields) || !domain.readings || typeof domain.readings !== 'object' || Array.isArray(domain.readings)) return null;
  const fields = [], ids = new Set();
  for (const row of domain.fields) {
    if (!row || !text(row.id) || !text(row.label) || !GROUPS.includes(row.group)
      || !validInspectorDescriptor(row) || row.id !== row.origin.schema_path || ids.has(row.id)
      || (row.action_panel != null && row.action_panel !== 'presentation')
      || (row.live_mutability === 'named-action') !== (row.action_panel === 'presentation')) return null;
    ids.add(row.id); fields.push({ ...row, descriptor: row });
  }
  const readings = new Map();
  for (const [id, reading] of Object.entries(domain.readings)) {
    if (!text(id) || !reading || !text(reading.label) || !['ship', 'sound', 'asset', 'catalog'].includes(reading.kind)
      || (reading.ship_id != null && !text(reading.ship_id)) || typeof reading.action_available !== 'boolean'
      || !reading.values || typeof reading.values !== 'object' || Array.isArray(reading.values)) return null;
    const values = new Map();
    for (const [key, item] of Object.entries(reading.values)) {
      if (!text(item) || !ids.has(schemaPath(key))) return null;
      values.set(key, item);
    }
    readings.set(id, { label: reading.label, kind: reading.kind, shipId: reading.ship_id || null,
      actionAvailable: reading.action_available, values });
  }
  return { fields, readings };
}

export function createGmPresentationInspectorPanel({ doc = globalThis.document, t = id => id,
  focusPresentation = null } = {}) {
  const el = suffix => doc?.getElementById(`gm-presentation-fields-${suffix}`);
  let domain = null, selected = null, gone = false;
  const session = createInspectorSession(GM_PRESENTATION_INSPECTOR_HISTORY_LIMIT);
  function choose(id, { record = true } = {}) {
    if (!id) return;
    session.forget(session.select(id, { record }));
    render();
  }
  function action(row, field, reading, id, value) {
    if (field.action_panel !== 'presentation') return;
    const button = doc.createElement('button'); button.type = 'button';
    button.dataset.inspectorAction = id; button.textContent = t('inspector.presentation.open_action');
    button.disabled = gone || !reading.actionAvailable || typeof focusPresentation !== 'function';
    button.addEventListener('click', () => {
      if (!button.disabled) focusPresentation({ reading, field: id, value });
    });
    row.append(button);
  }
  function render() {
    selected = session.selected;
    refreshNavigation();
    const live = domain?.readings.get(selected);
    if (live) { session.remember(selected, live); gone = false; }
    else gone = !!selected && session.reading(selected) != null;
    const reading = live || session.reading(selected);
    const select = el('subject');
    if (select && domain) {
      const options = [...domain.readings].map(([id, item]) => Object.assign(doc.createElement('option'), {
        value: id, textContent: `${t(`inspector.presentation.kind.${item.kind}`)} · ${item.label}`, selected: id === selected,
      }));
      if (gone) options.push(Object.assign(doc.createElement('option'), { value: selected,
        textContent: reading.label, selected: true, disabled: true }));
      select.replaceChildren(...options);
    }
    if (el('empty')) el('empty').hidden = !!reading;
    if (el('card')) el('card').hidden = !reading;
    if (!reading) return;
    el('status').textContent = t(gone ? 'inspector.presentation.subject_gone' : 'inspector.presentation.subject', { name: reading.label });
    el('status').dataset.gone = String(gone);
    const list = el('list'); list.replaceChildren(); let currentGroup = null;
    for (const field of domain.fields) {
      const matches = [...reading.values].filter(([id]) => schemaPath(id) === field.id);
      if (!matches.length) continue;
      if (!currentGroup || currentGroup.dataset.group !== field.group) {
        currentGroup = doc.createElement('section'); currentGroup.dataset.group = field.group;
        const heading = doc.createElement('h3'); heading.textContent = t(`inspector.presentation.group.${field.group}`);
        currentGroup.append(heading); list.append(currentGroup);
      }
      for (const [id, value] of matches) {
        const row = doc.createElement('div'); row.dataset.field = id;
        const label = doc.createElement('label'); label.textContent = `${t(field.label)} — ${id}`;
        const slot = doc.createElement('div');
        renderInspectorReadOnly(slot, value, field.descriptor, { t, label: field.label });
        slot.querySelector('input')?.setAttribute('aria-label', label.textContent);
        row.append(label, slot); action(row, field, reading, id, value); currentGroup.append(row);
      }
    }
  }
  el('subject')?.addEventListener('change', () => choose(el('subject').value));
  const refreshNavigation = bindInspectorNavigation({ session,
    back: el('back'), forward: el('forward'), status: el('status'), render,
  });
  return {
    update(payload) { const next = parseGmPresentationInspectorPayload(payload); if (!next) return false;
      for (const [id, reading] of next.readings) session.remember(id, reading); domain = next;
      if (!selected) choose(next.readings.keys().next().value); else render(); return true; },
    select(entity) { if (entity?.kind === 'player_ship') choose(`ship:${entity.entity_id}`); },
    reset() { domain = null; selected = null; gone = false; session.reset(); el('list')?.replaceChildren(); render(); },
    state: () => ({ ...session.state(), gone }),
  };
}
