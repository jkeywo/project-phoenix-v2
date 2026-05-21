/**
 * override-view.js
 *
 * Renders the resolved-template + override-summary UI inside the right-hand
 * properties pane.  Pure DOM rendering on top of OverrideEditor — see
 * editor/override-editor.js for the wire-format / API.
 *
 * Scope limits (per Slice 3 PRD #350):
 *   - Primitive leaves (number, string, boolean) are click-to-edit inline.
 *   - Arrays + nested objects show as read-only JSON.  Editing them inline
 *     is out of scope for Slice 3 (gold-plating); users may still attach
 *     overrides to nested leaves by clicking deeper into the tree (we
 *     recurse one level of plain objects so e.g. `hull.max` is reachable).
 *   - Arrays merge as REPLACE (not concat); this matches OverrideEditor.
 *
 * Selection -> render flow:
 *   1. sidebar.js calls renderOverridePanel(spawn, layer, {canvasManager}).
 *   2. We look up the template via entity-cache.  If missing, we render a
 *      placeholder and kick off an async load + re-render.
 *   3. Build OverrideEditor(template), re-apply each leaf of spawn.override.
 *   4. Render resolved fields and the summary card.
 *
 * Edits write back to `spawn.override` and call `canvasManager.renderAll()`
 * to keep the V1 mutation contract (snapshot -> mutate -> dirty -> render).
 */

import { OverrideEditor } from './override-editor.js';
import { getEntityConfig, loadEntityConfig } from './entity-cache.js';
import { snapshotForUndo } from './undo-controller.js';

// ── Helpers ──────────────────────────────────────────────────────────────────

function isPlainObject(v) {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

/** Walk a nested plain-object tree producing `{path, value}` leaves.
 *  Arrays are treated as leaves (REPLACE-on-merge semantics). */
function flattenLeaves(obj, prefix = '') {
  const out = [];
  if (!isPlainObject(obj)) return out;
  for (const [k, v] of Object.entries(obj)) {
    const p = prefix ? `${prefix}.${k}` : k;
    if (isPlainObject(v)) {
      out.push(...flattenLeaves(v, p));
    } else {
      out.push({ path: p, value: v });
    }
  }
  return out;
}

function formatValue(v) {
  if (typeof v === 'string') return v;
  if (typeof v === 'number' || typeof v === 'boolean') return String(v);
  return JSON.stringify(v);
}

/** Parse a string back into a value matching the type of `originalValue`.
 *  Returns `{ ok, value, error }`. */
function parseValue(raw, originalValue) {
  if (typeof originalValue === 'number') {
    const n = parseFloat(raw);
    if (Number.isNaN(n)) return { ok: false, error: 'not a number' };
    return { ok: true, value: n };
  }
  if (typeof originalValue === 'boolean') {
    if (raw === 'true') return { ok: true, value: true };
    if (raw === 'false') return { ok: true, value: false };
    return { ok: false, error: 'expected true/false' };
  }
  if (Array.isArray(originalValue) || isPlainObject(originalValue)) {
    try { return { ok: true, value: JSON.parse(raw) }; }
    catch (e) { return { ok: false, error: e.message }; }
  }
  // string fallback
  return { ok: true, value: raw };
}

function escapeHtml(s) {
  return String(s ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;');
}

// ── Public entry point ───────────────────────────────────────────────────────

/**
 * Render the override panel into `hostEl` for `spawn` whose template lives at
 * `spawn.template_path`.
 *
 * @param {HTMLElement} hostEl
 * @param {object}      spawn         Owns optional `.override` block.
 * @param {object}      layer         V1 layer record (for undo snapshot + isDirty).
 * @param {object}      deps
 * @param {object}      deps.canvasManager  Carries `renderAll()`.
 */
export function renderOverridePanel(hostEl, spawn, layer, deps) {
  if (!hostEl) return;
  hostEl.innerHTML = '';

  const templatePath = spawn?.template_path;
  if (!templatePath) {
    hostEl.innerHTML = '<p class="placeholder">No template_path on this spawn</p>';
    return;
  }

  let template = getEntityConfig(templatePath);
  if (!template) {
    hostEl.innerHTML = '<p class="placeholder">Loading template…</p>';
    loadEntityConfig(templatePath).then((tpl) => {
      if (!tpl) {
        hostEl.innerHTML = `<p class="placeholder">Template not found: ${escapeHtml(templatePath)}</p>`;
        return;
      }
      // Re-enter with the loaded template.
      renderOverridePanel(hostEl, spawn, layer, deps);
    });
    return;
  }

  // Build editor and replay existing overrides.
  const editor = new OverrideEditor(template);
  const existingLeaves = flattenLeaves(spawn.override ?? {});
  for (const { path, value } of existingLeaves) {
    editor.setOverride(path, value);
  }

  const onEdit = (path, newValue) => {
    snapshotForUndo(layer);
    editor.setOverride(path, newValue);
    spawn.override = editor.getOverrides();
    if (Object.keys(spawn.override).length === 0) delete spawn.override;
    layer.isDirty = true;
    deps.canvasManager.renderAll();
  };

  const onClear = (path) => {
    snapshotForUndo(layer);
    editor.clearOverride(path);
    spawn.override = editor.getOverrides();
    if (Object.keys(spawn.override).length === 0) delete spawn.override;
    layer.isDirty = true;
    deps.canvasManager.renderAll();
  };

  // Resolved-template form (read-only by default; click to edit a value).
  const resolved = editor.getResolvedView();
  const overrideKeys = new Set(editor.getOverridesSummary().map(s => s.path));

  const formPanel = document.createElement('div');
  formPanel.className = 'override-panel';
  formPanel.innerHTML = `<h4>RESOLVED TEMPLATE (${escapeHtml(shortPath(templatePath))})</h4>`;

  const leaves = flattenLeaves(resolved);
  if (leaves.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'override-summary-empty';
    empty.textContent = '(template has no scalar fields)';
    formPanel.appendChild(empty);
  } else {
    for (const { path, value } of leaves) {
      formPanel.appendChild(buildFieldRow(path, value, overrideKeys.has(path), onEdit));
    }
  }
  hostEl.appendChild(formPanel);

  // Summary card.
  hostEl.appendChild(buildSummaryCard(editor.getOverridesSummary(), onClear));
}

function buildFieldRow(path, value, isOverridden, onEdit) {
  const row = document.createElement('div');
  row.className = 'override-field' + (isOverridden ? ' overridden' : '');

  const label = document.createElement('span');
  label.className = 'of-label';
  label.textContent = path;
  row.appendChild(label);

  const valEl = document.createElement('span');
  valEl.className = 'of-value';
  valEl.textContent = formatValue(value);
  valEl.title = 'Click to override';
  row.appendChild(valEl);

  valEl.addEventListener('click', () => {
    const input = document.createElement('input');
    input.className = 'of-input';
    input.type = 'text';
    input.value = formatValue(value);
    input.spellcheck = false;
    valEl.replaceWith(input);
    input.focus();
    input.select();

    let committed = false;
    const cancel = () => {
      if (committed) return;
      committed = true;
      input.replaceWith(valEl);
    };
    const commit = () => {
      if (committed) return;
      committed = true;
      const parsed = parseValue(input.value, value);
      if (!parsed.ok) {
        // Discard and revert; surfacing a tooltip would be gold-plating
        // for Slice 3.  Log so devs can see why it didn't stick.
        console.warn(`[override-view] cannot parse "${input.value}" as ${typeof value}: ${parsed.error}`);
        input.replaceWith(valEl);
        return;
      }
      onEdit(path, parsed.value);
      // renderAll() triggered by onEdit will rebuild the form; no need
      // to restore valEl here.
    };

    input.addEventListener('blur', commit);
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') { e.preventDefault(); commit(); }
      else if (e.key === 'Escape') { e.preventDefault(); cancel(); }
    });
  });

  return row;
}

function buildSummaryCard(summary, onClear) {
  const card = document.createElement('div');
  card.className = 'override-summary-card';
  const h = document.createElement('h4');
  h.textContent = `OVERRIDES (${summary.length})`;
  card.appendChild(h);

  if (summary.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'override-summary-empty';
    empty.textContent = 'No overrides yet — click a field above to attach one.';
    card.appendChild(empty);
    return card;
  }

  for (const { path, value } of summary) {
    const row = document.createElement('div');
    row.className = 'override-summary-row';
    row.innerHTML = `<span class="osr-path">${escapeHtml(path)}</span>` +
                    `<span class="osr-value">${escapeHtml(formatValue(value))}</span>`;
    const btn = document.createElement('button');
    btn.className = 'osr-clear';
    btn.textContent = 'clear';
    btn.title = `Remove override at ${path}`;
    btn.addEventListener('click', () => onClear(path));
    row.appendChild(btn);
    card.appendChild(row);
  }
  return card;
}

function shortPath(p) {
  return p.split('/').pop() || p;
}
