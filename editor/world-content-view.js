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
 * @param {(name: string) => void} opts.onSelectEntity
 *        Called when a clickable row identifies an entity to highlight.
 */
export function renderWorldContentPanel({
  worldState,
  crossRefIndex,
  activeLayerPath,
  onSelectEntity,
}) {
  const host = document.getElementById('worldContentList');
  if (!host) return;

  if (!worldState) {
    host.innerHTML = '<p class="placeholder">Open a world to see its content</p>';
    return;
  }

  const data = getWorldContentData(worldState, crossRefIndex, activeLayerPath);

  host.innerHTML = '';
  for (const section of SECTIONS) {
    const rows = data[section.key] || [];
    host.appendChild(buildSection(section, rows, onSelectEntity));
  }
}

function buildSection(section, rows, onSelectEntity) {
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
    const fresh = buildSection(section, rows, onSelectEntity);
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
        body.appendChild(section.render(row, section.icon, onSelectEntity));
      }
    }
    wrap.appendChild(body);
  }
  return wrap;
}

function makeRow(icon, label, refCount, targetEntity, onSelectEntity) {
  const row = document.createElement('div');
  row.className = 'world-content-row';
  if (targetEntity) row.classList.add('clickable');
  row.innerHTML = `<span class="wc-icon">${icon}</span><span class="wc-label">${escapeHtml(label)}</span>` +
    (refCount != null ? `<span class="wc-refcount">(${refCount})</span>` : '');
  if (targetEntity && typeof onSelectEntity === 'function') {
    row.addEventListener('click', () => onSelectEntity(targetEntity));
  }
  return row;
}

function renderAnchorRow(row, icon, onSelectEntity) {
  // Anchors are not directly entity-clickable; show the count of spawns
  // anchored to them but no highlight (per audit §4 decision).
  return makeRow(icon, row.name, row.refCount, null, onSelectEntity);
}

function renderEntityRow(row, icon, onSelectEntity) {
  const tail = row.template_path ? ` ← ${shortPath(row.template_path)}` : '';
  return makeRow(icon, `${row.name}${tail}`, row.refCount, row.name, onSelectEntity);
}

function renderTriggerRow(row, icon, onSelectEntity) {
  const cond = row.condition || '*';
  const ent  = row.entity ? `:${row.entity}` : '';
  return makeRow(icon, `${cond}${ent} (×${row.actionCount})`, null, row.entity || null, onSelectEntity);
}

function renderCommsRow(row, icon, onSelectEntity) {
  const parts = [row.from, row.trigger].filter(Boolean).join(' / ');
  return makeRow(icon, parts || '(unnamed)', null, row.entity || null, onSelectEntity);
}

function renderObjectiveRow(row, icon, onSelectEntity) {
  const text = row.text ? `: ${row.text}` : '';
  return makeRow(icon, `${row.id}${text}`, row.refCount, null, onSelectEntity);
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
