/**
 * trigger-view.js
 *
 * Renders the trigger editor inside the right-hand properties pane.
 * Composed of:
 *   - A condition `<select>` (on_destroyed | on_attacked | on_timer | on_hailed)
 *   - An entity picker (hidden when condition === 'on_timer')
 *   - An `after_secs` input (shown only when condition === 'on_timer')
 *   - One `<action-card>` per action in the trigger
 *   - An "+ Add Action" composer
 *
 * All mutations follow the snapshot-BEFORE-mutate contract via
 * `snapshotForUndo(layer)`.  After each mutation `canvasManager.renderAll()`
 * is called so the World Content panel + canvas refresh.
 *
 * Scope: ACs #1, #2, #3, #4 of Slice 4a (PRD #350).  The file picker
 * surface is stubbed via `window.prompt`; Slice 4b will swap in a real
 * project-root-anchored file dialog.
 */

import { ACTION_SCHEMA } from './action-schema.js';
import { renderActionCard } from './action-card-view.js';
import { snapshotForUndo } from './undo-controller.js';

const TRIGGER_CONDITIONS = ['on_destroyed', 'on_attacked', 'on_timer', 'on_hailed'];

/**
 * Render a trigger editor.
 *
 * @param {HTMLElement} host
 * @param {object} selection      { type:'trigger', triggerIndex, layer }
 * @param {object} deps
 * @param {Array<{path, worldState}>} deps.allLayers
 * @param {object} deps.canvasManager  Provides `renderAll()`.
 * @param {object} [deps.layerManager]
 * @param {(rootPath: string) => Promise<string|null>} [deps.openFilePicker]
 *        Optional override (mainly for tests). Defaults to a window.prompt
 *        stub.
 */
export function renderTriggerPanel(host, selection, deps) {
  const layer = selection.layer;
  const triggerIndex = selection.triggerIndex;

  if (!layer || !layer.toml || !Array.isArray(layer.toml.trigger)) {
    host.innerHTML = '<p class="placeholder">Trigger no longer exists</p>';
    return;
  }
  const trigger = layer.toml.trigger[triggerIndex];
  if (!trigger) {
    host.innerHTML = '<p class="placeholder">Trigger no longer exists</p>';
    return;
  }

  const openFilePicker = deps.openFilePicker || defaultFilePicker;

  // Re-render the entire panel on any change.  Triggers are small; the
  // simplicity (and the need for state-dependent visibility — entity vs.
  // after_secs — driven by condition) outweighs the diff cost.
  const rerender = () => renderTriggerPanel(host, selection, deps);

  host.innerHTML = '';

  const panel = document.createElement('div');
  panel.className = 'trigger-panel';

  // ── Header ────────────────────────────────────────────────────────────
  const h4 = document.createElement('h4');
  h4.textContent = 'Trigger';
  panel.appendChild(h4);

  // Condition.
  panel.appendChild(makeConditionRow(trigger, layer, rerender));

  // Entity (hidden for on_timer).
  if (trigger.condition !== 'on_timer') {
    panel.appendChild(makeEntityRow(trigger, layer, deps, rerender));
  }

  // after_secs (only for on_timer).
  if (trigger.condition === 'on_timer') {
    panel.appendChild(makeAfterSecsRow(trigger, layer, rerender));
  }

  // ── Actions ───────────────────────────────────────────────────────────
  const actionsH4 = document.createElement('h4');
  actionsH4.textContent = 'Actions';
  panel.appendChild(actionsH4);

  const actionList = document.createElement('div');
  actionList.id = 'actionList';
  panel.appendChild(actionList);

  const actions = Array.isArray(trigger.action) ? trigger.action : [];
  actions.forEach((action, actionIndex) => {
    renderActionCard(actionList, action, {
      worldState: layer.toml,
      allLayers: deps.allLayers || [],
      onChange: (updated) => {
        snapshotForUndo(layer);
        ensureActionArray(trigger);
        trigger.action[actionIndex] = updated;
        markDirtyAndRender(layer, deps);
        rerender();
      },
      onMove: (direction) => {
        snapshotForUndo(layer);
        ensureActionArray(trigger);
        const arr = trigger.action;
        const j = direction === 'up' ? actionIndex - 1 : actionIndex + 1;
        if (j < 0 || j >= arr.length) return;
        [arr[actionIndex], arr[j]] = [arr[j], arr[actionIndex]];
        markDirtyAndRender(layer, deps);
        rerender();
      },
      onRemove: () => {
        snapshotForUndo(layer);
        ensureActionArray(trigger);
        trigger.action.splice(actionIndex, 1);
        markDirtyAndRender(layer, deps);
        rerender();
      },
      openFilePicker,
    });
  });

  // ── + Add Action ──────────────────────────────────────────────────────
  panel.appendChild(makeAddActionRow(trigger, layer, deps, rerender));

  host.appendChild(panel);
}

// ── Row builders ────────────────────────────────────────────────────────

function makeConditionRow(trigger, layer, rerender) {
  const group = document.createElement('div');
  group.className = 'property-group';

  const label = document.createElement('label');
  label.textContent = 'Condition:';
  group.appendChild(label);

  const select = document.createElement('select');
  select.id = 'trigCondition';
  for (const c of TRIGGER_CONDITIONS) {
    const opt = document.createElement('option');
    opt.value = c;
    opt.textContent = c;
    select.appendChild(opt);
  }
  // Allow the saved value through even if not in our known list.
  if (trigger.condition && !TRIGGER_CONDITIONS.includes(trigger.condition)) {
    const opt = document.createElement('option');
    opt.value = trigger.condition;
    opt.textContent = `${trigger.condition} (unknown)`;
    select.appendChild(opt);
  }
  select.value = trigger.condition ?? TRIGGER_CONDITIONS[0];
  select.addEventListener('change', (e) => {
    snapshotForUndo(layer);
    trigger.condition = e.target.value;
    layer.isDirty = true;
    rerender();
  });
  group.appendChild(select);
  return group;
}

function makeEntityRow(trigger, layer, deps, rerender) {
  const group = document.createElement('div');
  group.className = 'property-group';

  const label = document.createElement('label');
  label.textContent = 'Entity:';
  group.appendChild(label);

  const select = document.createElement('select');
  select.id = 'trigEntity';

  const empty = document.createElement('option');
  empty.value = '';
  empty.textContent = '(none)';
  select.appendChild(empty);

  // Gather every named entity from every open layer.
  const seen = new Set();
  for (const l of (deps.allLayers || [])) {
    const ws = l.worldState;
    if (!ws || !Array.isArray(ws.entity)) continue;
    for (const ent of ws.entity) {
      if (!ent.name || seen.has(ent.name)) continue;
      seen.add(ent.name);
      const opt = document.createElement('option');
      opt.value = ent.name;
      opt.textContent = ent.name;
      select.appendChild(opt);
    }
  }

  let unknown = false;
  if (trigger.entity && !seen.has(trigger.entity)) {
    unknown = true;
    const opt = document.createElement('option');
    opt.value = trigger.entity;
    opt.textContent = `${trigger.entity} ⚠ unknown`;
    select.appendChild(opt);
  }
  select.value = trigger.entity ?? '';

  select.addEventListener('change', (e) => {
    snapshotForUndo(layer);
    trigger.entity = e.target.value;
    layer.isDirty = true;
    rerender();
  });
  group.appendChild(select);

  if (unknown) {
    const warn = document.createElement('span');
    warn.className = 'action-field-warning';
    warn.textContent = '⚠ unknown';
    group.appendChild(warn);
  }
  return group;
}

function makeAfterSecsRow(trigger, layer, rerender) {
  const group = document.createElement('div');
  group.className = 'property-group';

  const label = document.createElement('label');
  label.textContent = 'After (s):';
  group.appendChild(label);

  const input = document.createElement('input');
  input.id = 'trigAfterSecs';
  input.type = 'number';
  input.step = '0.1';
  input.value = (trigger.after_secs ?? 0).toString();
  input.addEventListener('input', (e) => {
    const n = parseFloat(e.target.value);
    snapshotForUndo(layer);
    trigger.after_secs = Number.isNaN(n) ? 0 : n;
    layer.isDirty = true;
  });
  group.appendChild(input);
  return group;
}

function makeAddActionRow(trigger, layer, deps, rerender) {
  const wrap = document.createElement('div');
  wrap.className = 'action-add-row';

  const select = document.createElement('select');
  select.id = 'newActionType';
  for (const key of Object.keys(ACTION_SCHEMA)) {
    const opt = document.createElement('option');
    opt.value = key;
    opt.textContent = ACTION_SCHEMA[key].label;
    select.appendChild(opt);
  }
  wrap.appendChild(select);

  const btn = document.createElement('button');
  btn.type = 'button';
  btn.id = 'addActionBtn';
  btn.textContent = '+ Add Action';
  btn.addEventListener('click', () => {
    const type = select.value;
    const action = buildDefaultAction(type);
    if (!action) return;
    snapshotForUndo(layer);
    ensureActionArray(trigger);
    trigger.action.push(action);
    markDirtyAndRender(layer, deps);
    if (typeof rerender === 'function') rerender();
  });
  wrap.appendChild(btn);
  return wrap;
}

// ── Helpers ─────────────────────────────────────────────────────────────

function ensureActionArray(trigger) {
  if (!Array.isArray(trigger.action)) trigger.action = [];
}

function markDirtyAndRender(layer, deps) {
  layer.isDirty = true;
  if (typeof deps.canvasManager?.renderAll === 'function') {
    deps.canvasManager.renderAll();
  }
}

/**
 * Build a default-valued action for the given type.  Exported for the
 * integration test.
 */
export function buildDefaultAction(type) {
  const schema = ACTION_SCHEMA[type];
  if (!schema) return null;
  const action = { type };
  for (const f of schema.fields) {
    if (f.default !== undefined) {
      action[f.key] = f.default;
    } else if (f.required && f.enum) {
      action[f.key] = f.enum[0];
    } else if (f.required && f.type === 'string') {
      action[f.key] = '';
    } else if (f.required && f.type === 'number') {
      action[f.key] = 0;
    } else if (f.required && f.type === 'boolean') {
      action[f.key] = false;
    }
  }
  return action;
}

async function defaultFilePicker(root) {
  if (typeof window === 'undefined' || typeof window.prompt !== 'function') {
    return null;
  }
  const result = window.prompt('Enter world TOML path', root);
  if (result == null) return null;
  const trimmed = String(result).trim();
  return trimmed || null;
}
