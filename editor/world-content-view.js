/**
 * world-content-view.js
 *
 * Renders the five-section "World Content" tree in the editor's left
 * panel, sourced from `getWorldContentData(worldState, crossRefIndex,
 * activeLayerPath)`.
 *
 * Pure DOM rendering + click-event-dispatch.  Section collapse state is
 * preserved across re-renders via module-local Set.  Clicking a named
 * entity (or a trigger/comms row whose `entity` is set) dispatches
 * `onSelectEntity(name)`; otherwise rows are inert.
 *
 * Slice 4a addition: trigger rows additionally fire
 * `onSelectTrigger(triggerIndex)`, and comms rows fire
 * `onSelectComms(commsIndex)`.  Both callbacks are wired alongside
 * `onSelectEntity` (no replacement) — the right-hand pane picks which
 * takes precedence in its selection state.
 */

import { getWorldContentData } from './world-content-panel.js';

const collapsed = new Set();   // section keys that are collapsed

const SECTIONS = [
  { key: 'anchors',        label: 'Anchors',         icon: '◆', render: renderAnchorRow },
  { key: 'namedEntities',  label: 'Named entities',  icon: '◦', render: renderEntityRow },
  { key: 'triggers',       label: 'Triggers',        icon: '⚡', render: renderTriggerRow },
  { key: 'commsTemplates', label: 'Comms templates', icon: '💬', render: renderCommsRow },
  { key: 'objectives',     label: 'Objectives',      icon: '☑', render: renderObjectiveRow },
];

/**
 * Render (or re-render) the World Content panel into `#worldContentList`.
 * No-op if the host div is missing (Slice 1/2 callers without the new HTML).
 *
 * @param {object} opts
 * @param {object|null} opts.worldState          Active layer's toml object.
 * @param {object}      opts.crossRefIndex       CrossReferenceIndex instance.
 * @param {string|null} opts.activeLayerPath     Filename of the active layer.
 * @param {(name: string) => void}    [opts.onSelectEntity]
 *        Called when a clickable row identifies an entity to highlight.
 * @param {(triggerIndex: number) => void} [opts.onSelectTrigger]
 *        Called when a trigger row is clicked (Slice 4a). Fires in
 *        addition to `onSelectEntity` if the trigger has an entity.
 * @param {(commsIndex: number) => void} [opts.onSelectComms]
 *        Called when a comms row is clicked (Slice 4a stub for 4b).
 */
export function renderWorldContentPanel({
  worldState,
  crossRefIndex,
  activeLayerPath,
  onSelectEntity,
  onSelectTrigger,
  onSelectComms,
}) {
  const host = document.getElementById('worldContentList');
  if (!host) return;

  if (!worldState) {
    host.innerHTML = '<p class="placeholder">Open a world to see its content</p>';
    return;
  }

  const data = getWorldContentData(worldState, crossRefIndex, activeLayerPath);

  // Inject array indices so trigger/comms rows can dispatch index-based
  // selection callbacks even though the pure data module doesn't track them.
  (data.triggers || []).forEach((r, i) => { r.triggerIndex = i; });
  (data.commsTemplates || []).forEach((r, i) => { r.commsIndex = i; });

  const callbacks = { onSelectEntity, onSelectTrigger, onSelectComms };

  host.innerHTML = '';
  for (const section of SECTIONS) {
    const rows = data[section.key] || [];
    host.appendChild(buildSection(section, rows, callbacks));
  }
}

function buildSection(section, rows, callbacks) {
  const wrap = document.createElement('div');
  wrap.className = 'world-content-section';
  if (collapsed.has(section.key)) wrap.classList.add('collapsed');

  const header = document.createElement('div');
  header.className = 'world-content-section-header';
  header.textContent = `${collapsed.has(section.key) ? '▸' : '▾'} ${section.label} (${rows.length})`;
  header.addEventListener('click', () => {
    if (collapsed.has(section.key)) {
      collapsed.delete(section.key);
    } else {
      collapsed.add(section.key);
    }
    // Cheap local re-render: regenerate just this section.
    const fresh = buildSection(section, rows, callbacks);
    wrap.replaceWith(fresh);
  });
  wrap.appendChild(header);

  if (!collapsed.has(section.key)) {
    const body = document.createElement('div');
    body.className = 'world-content-section-body';
    if (rows.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'world-content-empty';
      empty.textContent = '(none)';
      body.appendChild(empty);
    } else {
      for (const row of rows) {
        body.appendChild(section.render(row, section.icon, callbacks));
      }
    }
    wrap.appendChild(body);
  }
  return wrap;
}

function makeRow(icon, label, refCount, clickHandler) {
  const row = document.createElement('div');
  row.className = 'world-content-row';
  if (clickHandler) row.classList.add('clickable');
  row.innerHTML = `<span class="wc-icon">${icon}</span><span class="wc-label">${escapeHtml(label)}</span>` +
    (refCount != null ? `<span class="wc-refcount">(${refCount})</span>` : '');
  if (clickHandler) {
    row.addEventListener('click', clickHandler);
  }
  return row;
}

function renderAnchorRow(row, icon, _callbacks) {
  // Anchors are not directly entity-clickable; show the count of spawns
  // anchored to them but no highlight (per audit §4 decision).
  return makeRow(icon, row.name, row.refCount, null);
}

function renderEntityRow(row, icon, callbacks) {
  const tail = row.template_path ? ` ← ${shortPath(row.template_path)}` : '';
  const handler = (typeof callbacks.onSelectEntity === 'function')
    ? () => callbacks.onSelectEntity(row.name)
    : null;
  return makeRow(icon, `${row.name}${tail}`, row.refCount, handler);
}

function renderTriggerRow(row, icon, callbacks) {
  const cond = row.condition || '*';
  const ent  = row.entity ? `:${row.entity}` : '';
  const handler = () => {
    if (typeof callbacks.onSelectTrigger === 'function') {
      callbacks.onSelectTrigger(row.triggerIndex);
    }
    if (row.entity && typeof callbacks.onSelectEntity === 'function') {
      callbacks.onSelectEntity(row.entity);
    }
  };
  return makeRow(icon, `${cond}${ent} (×${row.actionCount})`, null, handler);
}

function renderCommsRow(row, icon, callbacks) {
  const parts = [row.from, row.trigger].filter(Boolean).join(' / ');
  const handler = () => {
    if (typeof callbacks.onSelectComms === 'function') {
      callbacks.onSelectComms(row.commsIndex);
    }
    if (row.entity && typeof callbacks.onSelectEntity === 'function') {
      callbacks.onSelectEntity(row.entity);
    }
  };
  return makeRow(icon, parts || '(unnamed)', null, handler);
}

function renderObjectiveRow(row, icon, _callbacks) {
  const text = row.text ? `: ${row.text}` : '';
  return makeRow(icon, `${row.id}${text}`, row.refCount, null);
}

function shortPath(p) {
  return p.split('/').pop() || p;
}

function escapeHtml(s) {
  return String(s ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;');
}
