/** The entities/AI domain of the Live Inspector (issue #1489).
 *
 * A READING surface. Every field is rendered through the shared disabled
 * control, and the one field that has a named action — authored NPC doctrine —
 * offers a LINK to the panel that already owns that transaction rather than a
 * second form for it. There is no generic field setter here, deliberately: the
 * mutability classification is only worth publishing if nothing bypasses it.
 *
 * Gone/stale is decided by entity id, never by authored name. A hull that left
 * and a new hull authored with the same name are different entities, and
 * following the name would quietly re-point every reading at a stranger. A
 * reference hop follows a published id for the same reason.
 */
import { renderInspectorReadOnly, validInspectorDescriptor } from './inspector-field.js';

const text = value => typeof value === 'string';
const GROUPS = ['identity', 'placement', 'faction', 'tags', 'behaviour', 'ai', 'target', 'derived'];
/** How far back a reference hop may be retraced. Bounded because this is a
 * convenience for following a link, not a session history. */
export const GM_INSPECTOR_HISTORY_LIMIT = 20;

export function parseGmEntityInspectorPayload(payload) {
  let value = payload;
  if (typeof value === 'string') { try { value = JSON.parse(value); } catch (_) { return null; } }
  const domain = value?.entity_inspector;
  if (!domain || !Array.isArray(domain.fields) || !domain.readings
    || typeof domain.readings !== 'object' || Array.isArray(domain.readings)) return null;
  const fields = [];
  const seen = new Set();
  for (const field of domain.fields) {
    if (!field || !text(field.id) || !text(field.label) || !GROUPS.includes(field.group)
      || !validInspectorDescriptor(field) || field.id !== field.origin.schema_path
      || seen.has(field.id)) return null;
    seen.add(field.id);
    fields.push({ id: field.id, label: field.label, group: field.group, descriptor: field });
  }
  const readings = new Map();
  for (const [entityId, reading] of Object.entries(domain.readings)) {
    const values = reading?.values;
    if (!text(entityId) || !values || typeof values !== 'object' || Array.isArray(values)) return null;
    const entries = new Map();
    for (const [key, entry] of Object.entries(values)) {
      if (!text(key) || !text(entry) || !seen.has(key)) return null;
      entries.set(key, entry);
    }
    const references = new Map();
    const refs = reading.references;
    if (refs != null) {
      if (typeof refs !== 'object' || Array.isArray(refs)) return null;
      for (const [key, target] of Object.entries(refs)) {
        if (!text(key) || !text(target) || !seen.has(key)) return null;
        references.set(key, target);
      }
    }
    readings.set(entityId, { values: entries, references });
  }
  // Which entities the checked doctrine transaction can actually act on. A link
  // offered for a hull that action cannot touch is a control that leads
  // nowhere, so the panel reads the same projection the owning panel does.
  const doctrineTargets = new Set(
    value?.npc_doctrines && typeof value.npc_doctrines === 'object'
      ? Object.keys(value.npc_doctrines) : []);
  return { fields, readings, doctrineTargets };
}

const EMPTY_DOMAIN = () => ({ fields: [], readings: new Map(), doctrineTargets: new Set() });

export function createGmEntityInspectorPanel({ doc = globalThis.document, t = id => id,
  focusDoctrine = null, selectEntity = null } = {}) {
  const el = suffix => doc?.getElementById(`gm-entity-fields-${suffix}`);
  let domain = EMPTY_DOMAIN();
  let selected = null;
  // The last reading seen for the selected entity, kept so a despawn freezes
  // what was on screen instead of blanking it.
  let retained = null;
  let gone = false;
  const history = [];
  let cursor = -1;
  const rendered = new Map();

  function pushHistory(entityId) {
    if (!entityId || history[cursor] === entityId) return;
    history.splice(cursor + 1);
    history.push(entityId);
    if (history.length > GM_INSPECTOR_HISTORY_LIMIT) history.shift();
    cursor = history.length - 1;
  }

  function groupNode(group) {
    let node = el(`group-${group}`);
    if (!node) {
      const list = el('list');
      if (!list) return null;
      node = doc.createElement('section');
      node.id = `gm-entity-fields-group-${group}`;
      const heading = doc.createElement('h3');
      heading.id = `gm-entity-fields-group-${group}-heading`;
      heading.textContent = t(`inspector.entity.group.${group}`);
      // Named, so the grouping is a region assistive technology can move
      // between rather than a visual cue only.
      node.setAttribute('role', 'group');
      node.setAttribute('aria-labelledby', heading.id);
      node.append(heading);
      list.append(node);
    }
    return node;
  }

  function fieldNode(field) {
    let node = rendered.get(field.id);
    if (node && node.isConnected) return node;
    const parent = groupNode(field.group);
    if (!parent) return null;
    node = doc.createElement('div');
    node.dataset.field = field.id;
    const label = doc.createElement('span');
    label.textContent = t(field.label);
    node.append(label);
    parent.append(node);
    rendered.set(field.id, node);
    return node;
  }

  /** The subject line: WHICH entity these ~50 rows are about.
   *
   * Without it the entity's name is one row among equals, and a Back/Forward
   * hop rewrites every row while announcing nothing — the reader is moved and
   * not told. This is the live region, so a hop is spoken. */
  function subject(values) {
    const status = el('status');
    if (!status) return;
    const name = values.get('name') || selected || '';
    status.textContent = !selected ? ''
      : t(gone ? 'inspector.entity.subject_gone' : 'inspector.entity.subject', { name });
    status.dataset.gone = String(gone);
  }

  function render() {
    const reading = retained || { values: new Map(), references: new Map() };
    const values = reading.values;
    const empty = el('empty');
    if (empty) empty.hidden = !!selected;
    const card = el('card');
    if (card) card.hidden = !selected;
    subject(values);
    for (const field of domain.fields) {
      const node = fieldNode(field);
      if (!node) continue;
      const value = values.get(field.id);
      // A field the entity never authored is absent rather than blank: the
      // difference between "not authored" and "authored empty" is a fact.
      node.hidden = !selected || value === undefined;
      if (node.hidden) continue;
      let slot = node.querySelector('[data-inspector-value]');
      if (!slot) {
        slot = doc.createElement('div');
        slot.dataset.inspectorValue = '';
        node.append(slot);
      }
      renderInspectorReadOnly(slot, value, field.descriptor, { t, label: field.label });
      const input = slot.querySelector('input');
      if (input) { input.disabled = true; input.readOnly = true; }
      referenceHop(node, field, reading.references.get(field.id));
      if (field.descriptor.live_mutability === 'named-action') linkAction(node, field);
    }
    const back = el('back');
    const forward = el('forward');
    if (back) back.disabled = cursor <= 0;
    if (forward) forward.disabled = cursor < 0 || cursor >= history.length - 1;
  }

  /** Follow a reading that names another live object.
   *
   * The hop carries the published id, never the displayed name — the same rule
   * that decides gone/stale — so it cannot land on a replacement that happens
   * to share a name. A reference whose target is no longer live offers no hop. */
  function referenceHop(node, field, targetId) {
    let hop = node.querySelector('[data-inspector-reference]');
    const available = !!targetId && !!selectEntity && domain.readings.has(targetId);
    if (!hop) {
      if (!available) return;
      hop = doc.createElement('button');
      hop.type = 'button';
      hop.dataset.inspectorReference = field.id;
      hop.textContent = t('inspector.entity.follow');
      hop.addEventListener('click', () => {
        const target = hop.dataset.inspectorTarget;
        if (hop.disabled || !target) return;
        // The desk's own selection owner drives this, so the map, the entity
        // card and every other selection-scoped panel move together rather
        // than this panel holding a private idea of what is selected.
        selectEntity(target);
      });
      node.append(hop);
    }
    hop.hidden = !available;
    hop.disabled = !available;
    if (targetId) hop.dataset.inspectorTarget = targetId;
  }

  /** The named action is reached, never reimplemented: this focuses the panel
   * that already owns the checked transaction, its confirmation and its
   * attribution. A disappeared entity, or one that action cannot act on,
   * offers no working link. */
  function linkAction(node, field) {
    let link = node.querySelector('[data-inspector-action]');
    if (!link) {
      link = doc.createElement('button');
      link.type = 'button';
      link.dataset.inspectorAction = field.id;
      link.textContent = t('inspector.entity.open_doctrine');
      link.addEventListener('click', () => { if (!link.disabled) focusDoctrine?.(selected); });
      node.append(link);
    }
    link.disabled = gone || !focusDoctrine || !domain.doctrineTargets.has(selected);
  }

  function show(entityId, { record = true, focus = false } = {}) {
    selected = entityId || null;
    if (record) pushHistory(selected);
    const reading = selected ? domain.readings.get(selected) : null;
    if (reading) { retained = reading; gone = false; }
    else if (!selected) { retained = null; gone = false; }
    else gone = true;
    render();
    // A retraced hop rewrites every row, so the reader is put back at the
    // subject line rather than left on a button whose meaning just changed.
    if (focus) el('status')?.focus?.();
  }

  el('back')?.addEventListener('click', () => {
    if (cursor > 0) { cursor -= 1; show(history[cursor], { record: false, focus: true }); }
  });
  el('forward')?.addEventListener('click', () => {
    if (cursor >= 0 && cursor < history.length - 1) {
      cursor += 1;
      show(history[cursor], { record: false, focus: true });
    }
  });

  return {
    /** Fold one absolute projection. A malformed payload is rejected whole:
     * half a descriptor table is worse than the previous one. */
    update(payload) {
      const parsed = parseGmEntityInspectorPayload(payload);
      if (!parsed) return false;
      domain = parsed;
      show(selected, { record: false });
      return true;
    },
    select(entity) { show(entity?.entity_id || null); },
    reset() {
      domain = EMPTY_DOMAIN();
      selected = null; retained = null; gone = false;
      history.length = 0; cursor = -1;
      rendered.clear();
      el('list')?.replaceChildren();
      render();
    },
    state() {
      return { selected, gone, fields: domain.fields.length,
        history: [...history], cursor };
    },
  };
}
