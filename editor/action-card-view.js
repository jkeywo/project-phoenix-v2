/**
 * action-card-view.js
 *
 * Pure DOM-rendering module for a single trigger-action card.
 *
 * Exports `renderActionCard(host, action, deps) → void`.  The card is
 * schema-driven: it reads `ACTION_SCHEMA[action.type].fields` and maps
 * each field to an appropriate input element (text / number / checkbox /
 * select), with picker integrations for entity-name, objective-id,
 * AI-state, modifier-slot, flag-kind, and world-file path fields.
 *
 * The card never mutates `action` in place — every edit produces a fresh
 * `{...action, [key]: newValue}` object and calls `deps.onChange(updated)`.
 * The caller is responsible for writing the new object into world state
 * and re-rendering.
 *
 * Slice 4a contract (PRD #350):
 *   - AC #2: entity picker shows red ⚠ when the saved value is not in the
 *            options list, but still allows save.
 *   - AC #3: `complete_objective` / `fail_objective` use a dropdown for
 *            their `id` field, sourced from same-world `add_objective`s.
 *   - AC #4: `load_world` / `unload_world` (path) and `load_scenario`
 *            (load_scenario) fields use a file picker stub.
 */

import {
  ACTION_SCHEMA,
  INT_MODIFIER_SLOTS,
} from './action-schema.js';
import {
  getObjectiveIdOptions,
  getAiStateOptions,
  getModifierSlotOptions,
  getFlagKindOptions,
} from './trigger-pickers.js';
import { renderEntitySelect } from './entity-select-view.js';

/**
 * Render an action card into `host`.
 *
 * @param {HTMLElement} host
 * @param {object} action       The action object — must have a `type` key
 *                              matching an entry in ACTION_SCHEMA.
 * @param {object} deps
 * @param {object} deps.worldState
 * @param {Array<{path, worldState}>} deps.allLayers
 * @param {(updated: object) => void} deps.onChange
 * @param {(direction: 'up'|'down') => void} deps.onMove
 * @param {() => void} deps.onRemove
 * @param {(rootPath: string) => Promise<string|null>} deps.openFilePicker
 * @param {string}  [deps.basePath]     Validation-path prefix for the
 *                                      enclosing action array (e.g.
 *                                      `'trigger[3].action'`). When set,
 *                                      per-field rows tag themselves
 *                                      with `data-validation-path` so
 *                                      `applyValidationResults` can
 *                                      attach badges.
 * @param {number}  [deps.actionIndex]  Index of this card within its
 *                                      enclosing action array.
 */
export function renderActionCard(host, action, deps) {
  const schema = ACTION_SCHEMA[action.type];
  if (!schema) {
    host.innerHTML = `<div class="action-card"><div class="action-card-header">
      <span class="action-card-title">⚠ Unknown action type: ${escapeHtml(action.type)}</span>
    </div></div>`;
    return;
  }

  const card = document.createElement('div');
  card.className = 'action-card';

  // ── Header ────────────────────────────────────────────────────────────
  const header = document.createElement('div');
  header.className = 'action-card-header';

  const title = document.createElement('span');
  title.className = 'action-card-title';
  title.textContent = `▾ ${schema.label}`;
  header.appendChild(title);

  const controls = document.createElement('span');
  controls.className = 'action-card-controls';
  controls.appendChild(makeIconButton('⬆', 'up',     () => deps.onMove?.('up')));
  controls.appendChild(makeIconButton('⬇', 'down',   () => deps.onMove?.('down')));
  controls.appendChild(makeIconButton('✕', 'remove', () => deps.onRemove?.()));
  header.appendChild(controls);
  card.appendChild(header);

  // ── Body ──────────────────────────────────────────────────────────────
  const body = document.createElement('div');
  body.className = 'action-card-body';

  for (const field of schema.fields) {
    body.appendChild(renderField(action, field, deps));
  }

  card.appendChild(body);
  host.appendChild(card);
}

// ── Field rendering ─────────────────────────────────────────────────────

function renderField(action, field, deps) {
  const row = document.createElement('div');
  row.className = 'action-field';

  // Slice 7: stamp a validation path on every field row when the
  // caller threaded a basePath + actionIndex. The badge layer keys off
  // `data-validation-path`.
  if (typeof deps.basePath === 'string' && Number.isInteger(deps.actionIndex)) {
    row.dataset.validationPath =
      `${deps.basePath}[${deps.actionIndex}].${field.key}`;
  }

  const label = document.createElement('label');
  label.textContent = field.key;
  row.appendChild(label);

  const value = action[field.key];

  // Path-style fields (file picker).
  if (
    (field.key === 'path' && (action.type === 'load_world' || action.type === 'unload_world')) ||
    (field.key === 'load_scenario' && action.type === 'load_scenario')
  ) {
    row.appendChild(renderFilePickerField(action, field, value, deps));
    return row;
  }

  // Entity-name picker.
  if (field.key === 'entity') {
    renderEntitySelect(
      row,
      value,
      deps.allLayers || [],
      (newValue) => deps.onChange?.({ ...action, [field.key]: newValue }),
    );
    return row;
  }

  // set_ai_state: state dropdown depends on action.entity.
  if (field.key === 'state' && action.type === 'set_ai_state') {
    row.appendChild(renderAiStateSelect(action, field, value, deps));
    return row;
  }

  // complete_objective / fail_objective: id dropdown sourced from
  // same-world add_objective actions.
  if (field.key === 'id' && (action.type === 'complete_objective' || action.type === 'fail_objective')) {
    const opts = getObjectiveIdOptions(deps.worldState || {});
    row.appendChild(renderObjectiveIdSelect(action, field, value, opts, deps));
    return row;
  }

  // Modifier-slot dropdowns.
  if (field.key === 'slot') {
    const isInt = action.type === 'apply_int_modifier' || action.type === 'remove_int_modifier';
    const opts = isInt
      ? INT_MODIFIER_SLOTS.map((s) => ({ value: s, label: s }))
      : getModifierSlotOptions();
    row.appendChild(renderEnumSelect(action, field, value, opts, deps));
    return row;
  }

  // Flag-kind dropdown.
  if (field.key === 'kind' && (action.type === 'apply_flag' || action.type === 'remove_flag')) {
    const opts = getFlagKindOptions();
    row.appendChild(renderEnumSelect(action, field, value, opts, deps));
    return row;
  }

  // Generic schema-driven inputs.
  if (field.enum) {
    const opts = field.enum.map((v) => ({ value: v, label: v }));
    row.appendChild(renderEnumSelect(action, field, value, opts, deps));
    return row;
  }

  if (field.type === 'boolean') {
    row.appendChild(renderBooleanInput(action, field, value, deps));
    return row;
  }

  if (field.type === 'number') {
    row.appendChild(renderNumberInput(action, field, value, deps));
    return row;
  }

  // game_over message → textarea.
  if (field.key === 'message' && action.type === 'game_over') {
    row.appendChild(renderTextarea(action, field, value, deps));
    return row;
  }

  row.appendChild(renderTextInput(action, field, value, deps));
  return row;
}

// ── Individual input renderers ──────────────────────────────────────────

function renderAiStateSelect(action, field, value, deps) {
  const wrap = document.createElement('span');
  wrap.style.display = 'flex';
  wrap.style.flex = '1';

  const opts = getAiStateOptions(deps.worldState || {}, action.entity);

  if (opts.length === 0) {
    const hint = document.createElement('span');
    hint.className = 'action-field-hint';
    hint.textContent = 'Select an entity first';
    wrap.appendChild(hint);
    return wrap;
  }

  const select = document.createElement('select');
  const known = new Set();
  for (const o of opts) {
    known.add(o.value);
    const opt = document.createElement('option');
    opt.value = o.value;
    opt.textContent = o.label;
    select.appendChild(opt);
  }
  if (value && !known.has(value)) {
    const opt = document.createElement('option');
    opt.value = value;
    opt.textContent = `${value} (unknown)`;
    select.appendChild(opt);
  }
  select.value = value ?? '';

  select.addEventListener('change', (e) => {
    deps.onChange?.({ ...action, [field.key]: e.target.value });
  });
  wrap.appendChild(select);
  return wrap;
}

function renderObjectiveIdSelect(action, field, value, opts, deps) {
  const select = document.createElement('select');

  const known = new Set();
  for (const o of opts) {
    known.add(o.value);
    const opt = document.createElement('option');
    opt.value = o.value;
    opt.textContent = o.label;
    select.appendChild(opt);
  }
  if (value && !known.has(value)) {
    const opt = document.createElement('option');
    opt.value = value;
    opt.textContent = `${value} (unknown)`;
    select.appendChild(opt);
  } else if (!value) {
    const opt = document.createElement('option');
    opt.value = '';
    opt.textContent = '(unknown)';
    select.appendChild(opt);
  }
  select.value = value ?? '';

  select.addEventListener('change', (e) => {
    deps.onChange?.({ ...action, [field.key]: e.target.value });
  });
  return select;
}

function renderEnumSelect(action, field, value, opts, deps) {
  const select = document.createElement('select');
  const known = new Set();
  for (const o of opts) {
    known.add(o.value);
    const opt = document.createElement('option');
    opt.value = o.value;
    opt.textContent = o.label;
    select.appendChild(opt);
  }
  if (value !== undefined && value !== null && value !== '' && !known.has(value)) {
    const opt = document.createElement('option');
    opt.value = value;
    opt.textContent = `${value} (unknown)`;
    select.appendChild(opt);
  }
  select.value = value ?? '';
  select.addEventListener('change', (e) => {
    deps.onChange?.({ ...action, [field.key]: e.target.value });
  });
  return select;
}

function renderBooleanInput(action, field, value, deps) {
  const input = document.createElement('input');
  input.type = 'checkbox';
  input.checked = !!value;
  input.addEventListener('change', (e) => {
    deps.onChange?.({ ...action, [field.key]: e.target.checked });
  });
  return input;
}

function renderNumberInput(action, field, value, deps) {
  const input = document.createElement('input');
  input.type = 'number';
  input.step = '0.1';
  input.value = (value === undefined || value === null) ? '' : String(value);
  input.addEventListener('input', (e) => {
    const raw = e.target.value;
    if (raw === '') {
      deps.onChange?.({ ...action, [field.key]: 0 });
      return;
    }
    const n = parseFloat(raw);
    if (!Number.isNaN(n)) {
      deps.onChange?.({ ...action, [field.key]: n });
    }
  });
  return input;
}

function renderTextInput(action, field, value, deps) {
  const input = document.createElement('input');
  input.type = 'text';
  input.value = value ?? '';
  input.addEventListener('input', (e) => {
    deps.onChange?.({ ...action, [field.key]: e.target.value });
  });
  return input;
}

function renderTextarea(action, field, value, deps) {
  const ta = document.createElement('textarea');
  ta.rows = 3;
  ta.value = value ?? '';
  ta.addEventListener('input', (e) => {
    deps.onChange?.({ ...action, [field.key]: e.target.value });
  });
  return ta;
}

function renderFilePickerField(action, field, value, deps) {
  const wrap = document.createElement('span');
  wrap.style.display = 'flex';
  wrap.style.flex = '1';
  wrap.style.gap = '6px';
  wrap.style.alignItems = 'center';

  const display = document.createElement('span');
  display.className = 'action-field-hint';
  display.style.flex = '1';
  display.textContent = value || '(no file selected)';
  wrap.appendChild(display);

  const btn = document.createElement('button');
  btn.type = 'button';
  btn.textContent = 'Pick…';
  btn.addEventListener('click', async () => {
    const picker = deps.openFilePicker;
    if (typeof picker !== 'function') return;
    try {
      const chosen = await picker('assets/worlds/');
      if (chosen && typeof chosen === 'string') {
        deps.onChange?.({ ...action, [field.key]: chosen });
      }
    } catch (err) {
      console.warn('[action-card-view] file picker failed:', err?.message || err);
    }
  });
  wrap.appendChild(btn);
  return wrap;
}

// ── Helpers ─────────────────────────────────────────────────────────────

function makeIconButton(label, dataAction, onClick) {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'btn-icon';
  btn.dataset.action = dataAction;
  btn.textContent = label;
  btn.addEventListener('click', onClick);
  return btn;
}

function escapeHtml(s) {
  return String(s ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;');
}
